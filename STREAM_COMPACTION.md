# Stream Compaction Before Download

We are working on a GPU-accelerated stock market agent simulation in Rust
(/workspace/workspace/bigsoup). The simulation runs 1M agents per tick using two CUDA kernels
(agent_decide → classify_orders), then downloads classified output arrays to the CPU for LOB
matching.

## Problem

The classify kernel writes 5 output arrays of size N (1M agents):

    d_out_order_type[N]    i32   (0=skip, 1=limit_buy, 2=limit_sell, 3=market_buy, 4=market_sell)
    d_out_bid_price[N]     f32
    d_out_ask_price[N]     f32   (nonzero only for MM requotes)
    d_out_qty[N]           f32
    d_out_cancel_flag[N]   i32   (1 = cancel this agent's resting orders)

With participation_threshold=0.1 only ~1% of agents produce a non-skip order each tick. The
remaining ~99% have order_type=0. We download all 20 MB every tick, but only ~200 KB of that is
useful. This wastes ~30% of wall time on PCIe transfer of zeros.

## Goal

Add a stream compaction step after the classify kernel that packs only the active entries (where
order_type != 0 OR cancel_flag != 0) into dense output buffers, then downloads only those dense
buffers plus a count.

## Proposed approach

### New kernel: `compact_orders` (in `kernels/compact.cu`)

1. **Flag array**: Already have `d_out_order_type` — an entry is active if `order_type != 0` OR
   `cancel_flag != 0`.

2. **Prefix sum**: Compute an exclusive prefix sum over the flag array to get write indices.
   Use a work-efficient parallel scan (Blelloch-style or use CUB's `DeviceScan::ExclusiveSum`
   if available). Since we're using NVRTC, a hand-written scan is simpler — or do a two-pass
   approach:
   - Pass 1: per-block count of active elements → block_counts[num_blocks]
   - Pass 2: prefix sum of block_counts (small array, can be done on CPU or single-block kernel)
   - Pass 3: scatter active elements using per-block prefix + intra-block scan offset

3. **Scatter into dense buffers**:
   ```
   d_compact_agent_id[M]     u32   (the agent index — needed to build LobOrder)
   d_compact_order_type[M]   i32
   d_compact_bid_price[M]    f32
   d_compact_ask_price[M]    f32
   d_compact_qty[M]          f32
   d_compact_cancel_flag[M]  i32
   ```
   where M = number of active entries (written to a device scalar `d_active_count`).

4. **Download**:
   - Download `d_active_count` (4 bytes) first
   - Then download only M entries from each of the 6 dense arrays

### Alternative: struct-of-arrays → array-of-structs compaction

Instead of 6 separate dense arrays, pack each active entry into a single struct:
```c
struct CompactOrder {
    unsigned int agent_id;    // 4 bytes
    int   order_type;         // 4 bytes
    float bid_price;          // 4 bytes
    float ask_price;          // 4 bytes
    float qty;                // 4 bytes
    int   cancel_flag;        // 4 bytes
};  // 24 bytes per entry
```
This means a single contiguous download of M × 24 bytes. At ~10K active entries per tick that's
~240 KB — a single memcpy instead of 6. This is likely faster due to fewer API calls and better
PCIe utilization (one large transfer > many small ones).

### Changes to cuda_engine.rs

- Allocate dense output GPU buffers sized to N (worst case all active). They can be reused
  across ticks since M <= N.
- Allocate corresponding pinned host buffers (PinnedReadableSlice).
- After classify kernel launch, launch compact kernel on same stream.
- Download d_active_count, synchronize, then download only the first M entries.
  - For the partial download, use a memcpy with byte count = M * sizeof(T) rather than
    downloading the full buffer. Check if cudarc's memcpy_dtoh supports partial copies;
    if not, use the raw `cuMemcpyDtoHAsync` with explicit byte counts.
- The CPU loop that builds LobOrders iterates 0..M instead of 0..N, reading agent_id from the
  compacted array.

### Changes to classify kernel

None — it stays the same. The compact kernel reads its output.

### Buffers to remove from download

After compaction works:
- Stop downloading d_out_order_type, d_out_bid_price, d_out_ask_price, d_out_qty,
  d_out_cancel_flag as full N-sized arrays.
- The h_out_* PinnedReadableSlice buffers can be removed (replaced by the compact versions).

### Performance estimate

- PCIe transfer: 20 MB → ~240 KB per tick (100x reduction)
- Compact kernel: one pass over N elements, similar cost to classify (~1ms at 1M agents)
- Net saving: ~2s per 1000 ticks (from current ~2.3s download phase)
- The CPU-side order building loop also speeds up: iterating ~10K entries instead of 1M

### Key files

- `src/engine/cuda_engine.rs` — main file to modify
- `kernels/classify.cu` — existing classify kernel (do not modify)
- `kernels/compact.cu` — new file for the compaction kernel
- `src/engine/mod.rs` — no changes needed
- `src/sim.rs` — no changes needed

### Implementation notes

- Use NVRTC to compile compact.cu (same pattern as classify.cu in cuda_engine.rs).
- Launch on the same stream after classify, before any download. No synchronize between
  classify and compact.
- The d_active_count scalar should be a CudaSlice<u32> of length 1. Download it to a
  pinned host u32, synchronize, then use the value to size the subsequent downloads.
- For the struct-of-arrays approach: the 6 dense arrays can share a single large allocation
  if desired, but separate allocations are simpler and fine for a first pass.
- Run `cargo test` after implementation — there are 24 tests including smoke tests that check
  determinism.
