# econsim Design Choices

This document maps the key architectural decisions, the options for each, and the tradeoffs between simulation speed and kernel expressiveness / model complexity.

---

## 1. Memory Layout

| Option | Speed | Expressiveness | Notes |
|--------|-------|---------------|-------|
| **SoA (current)** | Fast — coalesced GPU reads, SIMD-friendly | Moderate — fixed-width param vectors | Each field is a flat `Vec<f32>` / `CudaSlice<f32>`. GPU threads reading the same field hit consecutive memory addresses. |
| **AoS** | Slow on GPU — strided access pattern kills memory bandwidth | High — agents can be arbitrary structs | Natural for CPU, terrible for GPU coalescing. Would need a transpose step. |
| **Hybrid SoA + variable-length** | Medium | High — agents can have different-sized strategy vectors | Use a fixed SoA core (position, cash) + a CSR-style indirection table for variable-length params. Adds complexity but allows heterogeneous agent models. |
| **Tiled SoA** | Fastest — exploits shared memory | Low-moderate | Group agents into tiles of 256, load a tile's params into shared memory, process, write back. Best throughput but harder to program. |

**Cache context:** Tiled SoA's advantage depends heavily on L2 size. At N=1M, the SoA memory footprint is:

```
position:        1M × 4 bytes  =  4 MB
cash:            1M × 4 bytes  =  4 MB
strategy_params: 1M × 8 × 4   = 32 MB   (K=8 params per agent)
internal_state:  1M × 4 × 4   = 16 MB   (M=4 state values per agent)
order_price:     1M × 4 bytes  =  4 MB
order_quantity:  1M × 4 bytes  =  4 MB
─────────────────────────────────────
total                            64 MB
```

The A100 has a 40MB L2 cache — position + cash + internal_state (24MB) fit comfortably, and strategy_params (32MB) nearly fits. On a 1080 Ti (2.75MB L2) nothing fits, so tiled SoA's locality benefit would be much more pronounced (estimated 20-30% improvement). On an H100 (50MB L2) plain SoA is even less of a concern.

**Recommendation:** SoA is the right starting point. If we want variable agent complexity later, the hybrid approach (fixed core + CSR indirection) adds it without breaking the fast path. Tiled SoA is a micro-optimization worth benchmarking only on small-L2 GPUs or at N > 10M where the working set overflows even large caches.

---

## 2. Agent Decision Model

This is the biggest design lever — it determines what kinds of models the system can express.

### Option A: Monolithic Kernel (current)
One CUDA kernel (`decide.cu`) contains all agent logic. All agents run the same code with different parameters.

- **Speed:** Maximum. No kernel launch overhead, no inter-kernel synchronization.
- **Expressiveness:** Low-moderate. Adding new signal types means editing the kernel. Agent "types" are just parameter vectors — a mean reverter is a trend follower with `trend_follow=0`.
- **Hot reload:** Easy — one file to watch, one kernel to swap.

### Option B: Multi-Kernel Pipeline
Break the decision into phases, each a separate kernel:
1. `observe.cu` — compute signals from market state
2. `combine.cu` — weighted combination of signals
3. `emit.cu` — convert signal to order price/quantity

- **Speed:** ~10-20% slower due to kernel launch overhead and extra global memory round-trips between phases.
- **Expressiveness:** High. Users can swap individual phases. Can add new signal kernels without touching others.
- **Hot reload:** Per-phase reload.

### Option C: Interpreted / Bytecode Agents
Define a small instruction set (stack machine or register machine). Each agent has a bytecode program stored in a device buffer. The kernel is a bytecode interpreter.

```
PUSH best_bid
PUSH best_ask
ADD
PUSH 0.5
MUL          ; mid = (bid + ask) / 2
LOAD_PARAM 0 ; aggression
MUL          ; order_price = mid * aggression
```

- **Speed:** 5-50x slower than native CUDA depending on program length. Branch divergence from different program lengths hurts badly.
- **Expressiveness:** Maximum. Arbitrary per-agent logic without recompilation.
- **Hot reload:** Change bytecode programs without touching CUDA at all.

### Option D: Expression Tree / Functional Composition
Define a fixed set of "signal functions" (EMA, mean reversion, momentum, etc.) and let users compose them via a DAG. The kernel evaluates the DAG.

- **Speed:** 2-5x slower than monolithic. Can be mitigated by JIT-compiling the DAG to CUDA source via NVRTC.
- **Expressiveness:** High within the vocabulary of built-in functions. Can't express truly novel logic without adding new primitives.
- **Hot reload:** DAG specification changes trigger NVRTC recompilation. Subsecond reload.

### Option E: NVRTC JIT from Python-specified Templates
Python specifies agent logic as string templates with parameters. Rust fills in the values, compiles via NVRTC at runtime.

```python
kernel_src = """
float signal = {mean_rev_weight} * (fair_est - mid)
             + {trend_weight} * (mid - ema);
"""
```

- **Speed:** Same as monolithic once compiled.
- **Expressiveness:** Very high — users write CUDA directly from Python, constrained only by the I/O interface.
- **Hot reload:** Natural — recompile on change.

---

## 3. Order Matching

| Option | Speed | Complexity | Realism |
|--------|-------|-----------|---------|
| **Clearing auction (current)** | Fast — O(N log N) sort + linear scan | Low | Low — all orders matched simultaneously, no time priority within a tick |
| **Persistent LOB** | Medium — O(N log N) insertions into price-level tree | Medium | High — resting orders, partial fills, realistic queue position |
| **GPU-sorted matching** | Fastest at large N — radix sort on GPU | High | Low-medium — still batch-oriented |
| **Continuous double auction (event-driven)** | Slow — sequential by nature | High | Highest — true price-time priority |

**Key tradeoff:** The clearing auction is ~100x simpler and sufficient to produce emergent price dynamics. A persistent LOB matters when studying queue position, market making P&L, or HFT strategies. If the research question is about macro dynamics (price discovery, volatility clustering), the clearing auction is fine.

**GPU-sorted matching** is worth benchmarking: use `cub::DeviceRadixSort` to sort orders by price on GPU, then do the matching walk on CPU. Avoids the O(N log N) CPU sort bottleneck at N > 1M.

---

## 4. State Ownership: CPU-Authoritative vs GPU-Authoritative

### CPU-Authoritative (current design)
Agent state lives in host memory. Uploaded to GPU before each step, orders downloaded after.

- **Speed:** O(N) PCIe transfers every tick. At N=1M with 14 floats/agent, that's ~56 MB round-trip. ~2ms on PCIe 3.0 x16.
- **Simplicity:** High. CPU always has current state. Easy to debug, snapshot, serialize.

### GPU-Authoritative
Agent state lives on device. CPU only sees it on-demand snapshots.

- **Speed:** Eliminates PCIe transfers on the hot path. Only the order buffer (2 floats/agent = ~8MB) needs to come back for CPU-side matching.
- **Simplicity:** Medium. Need explicit download calls for debugging/analysis. State divergence bugs are harder to catch.

### Split
Position/cash on CPU (modified by matching engine). Strategy params + internal state on GPU (modified by kernel). Only upload BBO (24 bytes) and download orders (8 bytes/agent).

- **Speed:** Best of both worlds. Matching engine updates positions on CPU. Kernel only needs params + internal state.
- **Complexity:** Medium-high. Two sources of truth — need careful sync when GPU needs updated positions.

**Recommendation:** Split ownership, reinforced by the precision argument. Cash and position live on CPU in f64 (accumulated by the fill loop). Strategy params and internal state live on GPU in f32 (read/written by the kernel). Per-tick transfers: upload BBO (24 bytes) + upload positions as f32 (4N bytes, narrowing cast) + download orders (8N bytes). Internal state only downloads on-demand for snapshots. This is both the fastest and most numerically correct option.

---

## 5. Fill Application

| Option | Description | Tradeoff |
|--------|-------------|----------|
| **CPU fills in f64 (recommended)** | Iterate trades, update `position[buyer] += qty` and `cash[buyer] -= price * qty` in f64 on CPU | Simple. Trades are O(N) worst case but typically << N. f64 accumulation avoids precision drift over long runs. |
| **GPU fills in f32** | Upload trade list to device, run a scatter kernel | Eliminates position re-upload, but accumulates rounding errors in cash/position. An agent with $10k doing $0.01 trades loses precision after ~100k ticks. |
| **GPU atomic fills** | Each agent atomically adds to its position from the order match | Requires `atomicAdd` on f32 — same precision problem, plus race conditions if one agent appears in multiple trades per tick. |

**CPU fills in f64 is the right default.** Cash and position are the only fields that accumulate unboundedly over the simulation lifetime — every other field is either static (params), self-correcting (EMA), or ephemeral (orders). This mirrors standard practice in backtesting: f64 portfolio accounting, f32 signal math.

The GPU kernel only *reads* position (for the risk aversion penalty) and doesn't need full precision — "am I long 47.3 units or 47.300003 units" doesn't change the decision. So the per-tick cost is: upload positions as f32 to device (a narrowing cast, 4N bytes), download orders (8N bytes). The f64 → f32 cast happens once per tick on upload and is free relative to the PCIe transfer.

This also **strengthens the case for CPU-side matching**: if fills must happen in f64 for precision, and f64 is impractical on most GPUs (1/32 to 1/64 throughput), then the matching + fill pipeline belongs on the CPU regardless of N.

---

## 6. RNG Strategy

| Option | Quality | Speed | Reproducibility |
|--------|---------|-------|----------------|
| **Per-agent LCG (current)** | Low — LCG has poor statistical properties, but adequate for noise perturbation | Fastest — no memory access, 2 multiplies | Fully deterministic per seed |
| **Per-agent xoshiro128+** | Good | Fast — 4 state words per agent (16 extra bytes/agent in SoA) | Deterministic |
| **cuRAND (device API)** | High | Medium — library overhead, state initialization | Deterministic with cuRAND ordering |
| **CPU-generated noise buffer** | Highest (can use any RNG) | Slow — O(N) upload per tick | Deterministic |

**The LCG is fine for the noise term** — it just needs to produce uncorrelated perturbations, not pass statistical test suites. If we ever need high-quality random sampling (e.g., Monte Carlo within the kernel), upgrade to xoshiro.

---

## 7. Tick Granularity

| Model | Description | Speed | Realism |
|-------|-------------|-------|---------|
| **Synchronous (current)** | All agents observe same BBO, all decide simultaneously, all orders matched in one batch | Fastest | Low — no information asymmetry within a tick |
| **Micro-ticks** | Within each tick, process agents in random-order batches of ~1000, updating BBO between batches | ~N/batch_size × slower | Medium — some agents see updated prices |
| **Asynchronous / event-driven** | Each agent has a Poisson arrival rate. Process events in time order. | Orders of magnitude slower (inherently sequential) | Highest |

**Micro-ticks are an interesting middle ground** worth testing. Run the kernel on sub-batches, update BBO between launches. This creates within-tick information asymmetry and more realistic price impact without going fully sequential.

---

## 8. Precision

| Type | Memory | Speed | When to use |
|------|--------|-------|-------------|
| **f32 (current)** | 4 bytes/value | Fastest on consumer GPUs (1080 Ti has 1/32 f64 rate) | Most ABM research |
| **f64** | 8 bytes/value | ~32x slower on 1080 Ti, ~2x slower on A100 | When studying numerical stability, long-horizon sims, or comparing to analytical results |
| **Fixed-point (i32 prices)** | 4 bytes | Same as f32 on GPU, exact arithmetic | Financial sims where rounding matters. Prices as integer cents avoid floating-point accumulation errors in cash. |

**f32 is correct for now.** If we ever see cash/position drift from accumulation errors over 100k+ ticks, consider fixed-point prices (multiply by 10000, store as i32).

---

## 9. Benchmarking Plan

To compare these choices empirically, we should measure:

| Metric | How |
|--------|-----|
| **Tick throughput** (ticks/sec at N=100k, 1M, 10M) | `Instant::now()` around `sim.run(1000)` |
| **Kernel time** | CUDA events around kernel launch |
| **PCIe transfer time** | CUDA events around memcpy |
| **Sort time** | Profile `process_orders` separately |
| **Price dynamics quality** | Autocorrelation of returns, volatility clustering (Hurst exponent), fat tails (kurtosis) |

The speed numbers tell us *what we can afford*. The dynamics quality tells us *what's worth affording*.

---

## Suggested Experiments

1. **Current foundation vs GPU-authoritative**: Benchmark PCIe overhead by implementing the GPU engine with positions staying on device (upload positions once, apply fills on GPU).

2. **Monolithic vs multi-kernel**: Split the current kernel into observe → combine → emit. Measure the launch overhead cost.

3. **Clearing auction vs GPU-sorted matching**: At N=1M, is the CPU sort the bottleneck? Try `thrust::sort` on the GPU.

4. **Micro-ticks**: Run 10 sub-batches of 10k agents per tick instead of 1 batch of 100k. Measure speed cost and dynamics improvement.

5. **NVRTC JIT from Python templates**: Prototype the Python → CUDA source → NVRTC → kernel pipeline. This could be the killer feature — researchers write agent logic in near-CUDA from a notebook.

---

## Recommended Path

For maximum research utility with minimum complexity:

1. **Keep the monolithic kernel + clearing auction** for v1. It's fast and produces interesting dynamics.
2. **Make the GPU engine GPU-authoritative** — avoid unnecessary PCIe round-trips.
3. **Add NVRTC JIT from Python templates** as the expressiveness lever — lets researchers modify agent logic without touching Rust.
4. **Defer persistent LOB and micro-ticks** until there's a research question that requires them.
5. **Benchmark at N=1M early** to identify the actual bottleneck before optimizing.
