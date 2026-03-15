# LOB Matching Performance

Baseline: 1M agents, 1000 ticks, GPU engine. LOB match = **479s (87%)**.

```
phase                             total        %
------------------------------------------------
exo price update               694.56us    0.00%
agent decide (total)              7.09s    1.29%
order convert                    23.44s    4.25%
lob match                       478.93s   86.93%
fill application                  5.21s    0.95%
lob expire                       34.69s    6.30%
------------------------------------------------
wall total                      550.96s  100.00%
```

That's ~479ms/tick for the match phase alone. Target: <100ms/tick.

---

## Where the time goes

At N=1M with ~20% MMs: ~1.2M orders/tick (MMs emit 2 each).

### 1. Per-order BTreeMap walk (~60% of match time)

Every `submit_order` iterates `asks.levels.iter_mut()` (or bids reversed) to find crossing prices. BTreeMap iteration is pointer-chasing through tree nodes — terrible cache locality at 1M calls/tick even when most orders cross 0-1 levels. The iterator must descend into the tree each time.

### 2. Per-order rest + index bookkeeping (~20%)

Every non-crossing order calls `rest_order` which does:
- `BTreeMap::entry()` — O(log P) tree traversal
- `HashMap::insert` into `order_index` — hash + probe
- `HashMap::entry` into `agent_orders` — hash + probe + SmallVec push

At 1M orders/tick that's ~3M hash operations just for resting.

### 3. Filled-order cleanup (~10%)

Each fill does `order_index.remove()` + `agent_orders.get_mut().retain()`. The SmallVec retain scans all entries for every single fill.

### 4. Trade Vec reallocation (~10%)

`submit_order` returns `Vec<Trade>`, then `process_tick` does `all_trades.extend(trades)`. At high fill rates this creates and extends millions of small Vecs.

---

## Optimization strategies (ordered by expected impact)

### A. Bucket array instead of BTreeMap (biggest win)

Replace `BTreeMap<OrderedFloat<f32>, PriceLevel>` with a fixed-size array indexed by price tick. If we define a tick size (e.g. 0.01) and center the array around midprice:

```rust
struct BucketBook {
    levels: Vec<Option<PriceLevel>>,  // fixed array, e.g. 10000 slots
    center: f32,                       // midprice
    tick_size: f32,                    // e.g. 0.01
}
```

- **Insert**: O(1) — compute index, push to VecDeque
- **Best bid/ask**: track with a single `i32` that advances on insert/remove
- **Match walk**: sequential array access from best price — perfect cache locality
- **Recenter**: shift the window when price drifts (amortized, rare)

Expected speedup: **5-10x** on the match phase. This is the standard HFT book structure.

### B. Pre-allocate trade buffer + batch submission

Instead of creating a new `Vec<Trade>` per `submit_order` call and extending into `all_trades`:

```rust
// In process_tick:
let mut trades = Vec::with_capacity(estimated_fills);

// In submit_order, pass &mut trades directly:
fn submit_order(&mut self, order: LobOrder, trades: &mut Vec<Trade>) { ... }
```

Eliminates ~1M small Vec allocations per tick. Expected: **~5-10% overall**.

### C. Batch cancel by tick instead of per-agent

Currently `expire_orders_before` does `retain()` on every VecDeque in every price level, plus per-order HashMap removes. With 1-tick TTL, nearly all orders expire.

Alternative: track orders by tick in a side structure:

```rust
tick_orders: HashMap<u64, Vec<(Side, OrderedFloat<f32>, u64)>>,  // tick -> [(side, price, order_id)]
```

On expire: remove the whole tick's entry, then batch-remove from price levels. Avoids scanning orders that won't expire.

Better yet: since TTL=1, just **clear the entire book** at expire time and only keep orders from the current tick. This is O(1) with a swap:

```rust
fn expire_all_before_current(&mut self) {
    // All orders from previous ticks are gone.
    // Only current-tick orders survive (placed in this tick's process_tick).
    // -> Just track which orders were placed this tick and rebuild.
}
```

Expected: **lob_expire drops from 35s to <1s**.

### D. Drop order_index and agent_orders entirely

These indexes exist to support `cancel_agent()` and filled-order cleanup. But:

- With 1-tick TTL, cancel_agent only needs to remove orders placed *this tick* by MMs. That's ~200k orders among ~600 price levels. A linear scan of just-placed MM order IDs is cheaper than maintaining two HashMaps across 1M+ entries.
- For filled-order cleanup, if we don't track order ownership in a HashMap, we skip 2M+ hash operations per tick.

Alternative: tag each `LobOrder` with agent_id (already done) and use a `HashSet<u32>` of cancelled agents. During matching, skip orders from cancelled agents:

```rust
// In match_buy_order:
if cancelled_agents.contains(&resting.agent_id) {
    level.orders.pop_front();
    continue;
}
```

Lazy cleanup — no index maintenance. Expected: **~15-20% of match time saved**.

### E. Rayon parallel order conversion

`convert_orders` at 23s (4.25%) processes 1M orders sequentially. This is embarrassingly parallel:

```rust
use rayon::prelude::*;
let (mm_orders, dir_orders): (Vec<_>, Vec<_>) = order_buffer
    .par_iter()
    .filter(|o| o.quantity.abs() > f32::EPSILON)
    .partition(|o| agents.agent_type[o.agent_id as usize] == 1);
```

Expected: **order_convert drops from 23s to ~6s** on 4 cores.

### F. Sort-and-sweep matching (alternative architecture)

Instead of submitting orders one at a time into a persistent book, batch all orders and sweep:

1. Separate into buys/sells (parallel partition)
2. Sort buys descending, sells ascending (parallel sort)
3. Two-pointer sweep to find all crosses
4. Resting orders = everything not matched

This is closer to the old clearing auction but with resting semantics. The sort is O(N log N) but with great cache locality (contiguous arrays). At 1M orders, a parallel radix sort on f32 prices takes ~5ms.

Expected: **match phase drops from 479s to ~20-40s** (dominated by sort, not tree ops).

---

## Recommended implementation order

| Priority | Optimization | Expected impact | Effort |
|----------|-------------|-----------------|--------|
| 1 | Bucket array book (A) | 5-10x match speedup | Medium |
| 2 | Pre-alloc trade buffer (B) | 5-10% overall | Small |
| 3 | Drop index HashMaps (D) | 15-20% match | Small |
| 4 | Batch expire / clear book (C) | lob_expire 35x faster | Small |
| 5 | Parallel convert (E) | order_convert 4x faster | Small |
| 6 | Sort-and-sweep (F) | Alternative to A, simpler | Medium |

Doing A + B + C + D together should bring the match phase from ~479ms/tick to **~30-60ms/tick**, well under the 100ms target.
