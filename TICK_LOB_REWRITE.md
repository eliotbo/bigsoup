# Task: Rewrite LOB BookSide to use tick-indexed Vec

## Goal

Replace the current sorted `Vec<PriceLevel>` in `src/market/lob.rs` with a flat `Vec<Option<PriceLevel>>` indexed by **cent tick** (0.01 price increments). This makes insert, lookup, and best-price access O(1) instead of O(n) or O(log n).

## Context

This is an agent-based market simulation with ~1M agents. Prices hover around 100.0. Market makers (MMs) post persistent two-sided quotes; directional agents post ephemeral 1-tick orders. The LOB's `process_tick` is the bottleneck — it handles cancels, matching, and resting ~10k orders per tick across 1000 ticks.

The current implementation uses a sorted `Vec<PriceLevel>` with binary search for insert and `Vec::insert`/`Vec::remove` for adding/removing price levels. This causes O(n) element shifting and is slower than the BTreeMap it replaced. We want the Vec approach but done correctly: **tick-indexed**, so there is no sorting or shifting at all.

## Tick size

**TICK_SIZE = 0.01** (one cent). All prices are snapped to the nearest cent on insert.

Price-to-index conversion:
```rust
const TICK_SIZE: f32 = 0.01;

fn price_to_tick(price: f32) -> i64 {
    (price / TICK_SIZE).round() as i64
}

fn tick_to_price(tick: i64) -> f32 {
    tick as f32 * TICK_SIZE
}
```

## Current file to rewrite: `src/market/lob.rs`

The current code is shown below. Rewrite it in place. Keep the same public API (`process_tick`, `submit_order_vec`, `cancel_agent`, `expire_orders_before`, `bbo`, `book_depth`, `bids_total_qty`, `asks_total_qty`, `book_bids`, `book_asks`).

### Current structures to replace

```rust
struct AgentEntry { agent_id: u32, qty: f32, tick: u64 }

struct PriceLevel {
    price: f32,
    total_quantity: f32,
    agents: Vec<AgentEntry>,
}

struct BookSide {
    levels: Vec<PriceLevel>,  // sorted, binary-searched — THIS IS THE PROBLEM
    ascending: bool,
}
```

### New structures

```rust
struct AgentEntry { agent_id: u32, qty: f32, tick: u64 }

struct PriceLevel {
    total_quantity: f32,
    agents: Vec<AgentEntry>,
}

struct BookSide {
    levels: Vec<Option<PriceLevel>>,  // indexed by (tick - base_tick)
    base_tick: i64,                    // tick value of levels[0]
    best_idx: Option<usize>,           // cached index of best occupied level
    ascending: bool,                   // true = asks (best = lowest), false = bids (best = highest)
}
```

### BookSide operations

**price_to_idx(price) -> usize**: snap price to tick, compute `(tick - base_tick) as usize`. If out of range, grow the Vec (extend with `None` on the appropriate end, adjusting `base_tick` if needed).

**insert(price, entry)**: O(1). Index into `levels`. If `None`, create `PriceLevel`. Push entry, add qty. Update `best_idx` if this index is better (lower for asks, higher for bids — but since index 0 direction depends on `ascending`, just compare properly).

**Matching (match_buy / match_sell)**: start at `best_idx`. Walk ticks one by one in the correct direction. Skip `None` levels. For each occupied level, pro-rata fill. When a level empties, set to `None`. After matching, update `best_idx` to the new first occupied level (either it's already at the right spot from the walk, or scan forward).

**cancel_batch**: single pass over all `Some` levels, retain agents not in cancel set. Set emptied levels to `None`. Update `best_idx`.

**expire**: same ephemeral_count short-circuit. Retain agents with `tick >= min_tick`. Set emptied levels to `None`. Update `best_idx`.

**best_idx maintenance**: after any operation that might empty the best level (matching, cancel, expire), scan from old `best_idx` toward worse prices to find the next `Some` level. This scan is bounded by the gap to the next level — typically 1-10 ticks.

### LimitOrderBook struct

```rust
pub struct LimitOrderBook {
    bids: BookSide,
    asks: BookSide,
    last_price: f32,
    tick: u64,
    ephemeral_count: usize,
}
```

No more `agent_orders`, `order_index`, `next_order_id`, `empty_prices_buf`, `filled_resting_buf`. The tick-indexed Vec replaces all of that.

### Initial sizing

Price starts at 100.0 = tick 10000. A reasonable initial range is ±10.0 (ticks 9000..11000), so `levels` starts as `vec![None; 2000]` with `base_tick = 9000`. If a price falls outside, grow dynamically.

### Key constants

```rust
const TICK_SIZE: f32 = 0.01;
const PERSISTENT_TICK: u64 = u64::MAX;  // MM orders that never expire
```

## What to keep unchanged

- **`src/market/types.rs`**: `LobOrder`, `Trade`, `BBO`, `Side`, `OrderType` — no changes. `LobOrder` still has `order_id` field (just ignored).
- **`src/sim.rs`**: the `process_tick` call site passes `&mut Vec<Trade>`, that stays the same. `convert_orders` unchanged.
- **Public API of `LimitOrderBook`**: same method signatures. `process_tick` takes `(cancel_agents, market_orders, limit_orders, tick, &mut Vec<Trade>)`. `submit_order_vec` returns `Vec<Trade>`. etc.
- **Pro-rata fill semantics**: when multiple agents rest at the same price, fill proportionally (`ratio = fill_qty / total_qty`, each agent fills `qty * ratio`). Use `swap_remove` for fully-filled agents.
- **Ephemeral count short-circuit**: `ephemeral_count` tracks non-persistent resting entries. `expire_orders_before` returns immediately when it's zero.
- **Batch cancel**: `process_tick` builds `FxHashSet<u32>` from cancel_agents and does one pass.

## What to keep from current tests

Port all existing tests. The fill semantics change from FIFO to pro-rata (already done). Test prices should still work after snapping to cent ticks (all test prices like 99.0, 99.5, 100.0, 100.5, 101.0 are already on-tick).

## Verify

- `cargo test` — all LOB tests + smoke tests pass
- No new warnings from lob.rs
- Run with 1M agents 1000 ticks: finite prices, positive volume, no panics
