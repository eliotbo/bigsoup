# econsim Handoff: What's Done, What's Next

## Project Overview

GPU-accelerated agent-based economic simulation library in Rust with a Python frontend. A continuous double auction with heterogeneous parameterized trading agents on a single asset.

Full spec is in `econ_sim_claude_code_prompt.md.pdf` in the repo root.

Architecture: CUDA kernels → Rust engine (cudarc) → Python frontend (PyO3).

## What Has Been Built (Steps 1-5)

All code compiles and passes 9 tests (`cargo test --release`).

### Source Files

| File | Purpose |
|------|---------|
| `src/market/types.rs` | `BBO`, `Order`, `Trade` structs |
| `src/agent/state.rs` | SoA `AgentState` — `position` and `cash` are `Vec<f64>` (precision), `strategy_params` and `internal_state` are `Vec<f32>`. Methods: `new(n,k,m)`, `randomize_params`, `get_param`, `set_param` |
| `src/market/order_book.rs` | Clearing auction order book with pluggable `SortStrategy` enum: `CpuSort`, `BucketSort { bucket_width }`, `CrossingOnly`. GPU radix sort is a planned 4th variant. |
| `src/engine/mod.rs` | `SimEngine` trait with `fn step(&mut self, agents, bbo, order_buffer) -> usize` |
| `src/engine/cpu_engine.rs` | `CpuEngine` — reference implementation. Decision logic is identical to the CUDA kernel spec (same math, same indexing). Casts `position` from f64→f32 for signal math. |
| `src/sim.rs` | `Simulation` struct with tick loop: BBO → engine step → match → apply fills (f64 accumulation) → record history. `SimConfig` struct. |
| `src/lib.rs` | Module root, re-exports `agent`, `engine`, `market`, `sim` |
| `src/main.rs` | Test binary: 100k agents, 4 archetypes, 1000 ticks |

### Tests

| File | Tests |
|------|-------|
| `src/agent/state.rs` | `test_new_allocates_zeroed`, `test_get_set_param`, `test_randomize_params` |
| `src/market/order_book.rs` | `test_basic_crossing`, `test_no_crossing`, `test_bbo_updates`, `test_sort_strategies_agree` |
| `tests/test_smoke.rs` | `test_smoke_1000_ticks` (no NaN/Inf), `test_deterministic` (same seed = same output) |

### Key Design Decisions Already Made

- **f64 for position/cash** (accumulation precision), **f32 for everything else** (signal math, orders, params, internal state). Fills happen CPU-side in f64. GPU kernel only reads position as f32 (narrowing cast on upload).
- **SoA layout** — flat arrays indexed by agent ID. No `Vec<Agent>` structs.
- **Order buffer pre-allocated** with capacity N, reused every tick.
- **All agents run the same code path** — different behavior emerges from different `strategy_params`, not if/else branches.
- **CPU-side matching is intentional** — order book stays on CPU. Don't GPU-accelerate it.

### Current Cargo.toml

```toml
[dependencies]
rand = "0.9"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

Only CPU dependencies. CUDA/PyO3 deps need to be added for steps 6-9.

## What Needs To Be Built (Steps 6-9)

### Step 8: CUDA Kernel (`kernels/decide.cu`)

The kernel is fully specified in the PDF (pages 7-8). Key points:

- Function signature: `extern "C" __global__ void agent_decide(float best_bid, float best_ask, float last_price, const float* position, const float* cash, const float* strategy_params, float* internal_state, float* order_price, float* order_quantity, int N, int K, int M)`
- Strategy params layout (K=8): `[aggression, mean_reversion, trend_follow, noise_scale, ema_alpha, fair_value_lr, position_limit, risk_aversion]`
- Internal state layout (M=4): `[fair_value_estimate, ema, prev_mid, rng_state]`
- Logic: update EMA, update fair value, compute mean-reversion + trend-following + LCG noise + position penalty signals, emit order price/quantity, clamp by position limits
- **The CPU engine (`src/engine/cpu_engine.rs`) already implements this exact logic** — the kernel must be a line-for-line mirror

Note: `position` is `Vec<f64>` on CPU but must be uploaded to GPU as `f32`. The kernel reads position as `const float*`.

### Step 9: CUDA Engine (`src/engine/cuda_engine.rs`)

Add to Cargo.toml: `cudarc = { version = "0.16", features = ["cuda-12040"] }` (the container has CUDA 12.4, driver 580.126.09, GTX 1080 Ti at device 0).

The `CudaEngine` struct (spec page 6):
- Holds `Arc<CudaDevice>`, device buffers mirroring AgentState fields, kernel function handle, config (n, k, m, block_size=256)
- `new(device_id, &AgentState)` — allocate device buffers, upload initial state, compile kernel via NVRTC
- `upload_agents(&AgentState)` — host→device (cast position/cash f64→f32 during upload)
- `download_agents(&mut AgentState)` — device→host (only called on-demand for snapshots)
- `reload_kernel(ptx)` — swap the function handle (for hot reload later)
- `step()` impl: upload BBO (constant memory or scalar args), upload positions as f32, launch kernel with grid=ceil(N/256) block=256, download order_price + order_quantity, construct `Vec<Order>` from the two arrays

### Step 6: PyO3 Bindings (`src/lib.rs`)

Add to Cargo.toml:
```toml
[lib]
name = "econsim"
crate-type = ["cdylib"]

[dependencies]
pyo3 = { version = "0.22", features = ["extension-module"] }
numpy = "0.22"

[build-dependencies]
pyo3-build-config = "0.22"
```

Add `pyproject.toml` for maturin build.

The `PySimulation` class (spec page 11):
- `#[new] fn new(config_json: &str)` — parse JSON config, create AgentState, randomize params, create engine (GPU or CPU based on config), create Simulation
- `fn step()`, `fn run(n_ticks)`
- `fn price_history() -> PyArray1<f32>`, `fn volume_history() -> PyArray1<f32>`
- `fn agent_positions() -> PyArray1<f64>`, `fn agent_cash() -> PyArray1<f64>` — note these are f64 now
- `fn bbo() -> (f32, f32, f32, f32, f32)`, `fn tick() -> u64`

The Rust `SimConfig` needs serde deserialization to accept the JSON from Python. The current `SimConfig` in `sim.rs` doesn't have serde derives yet.

### Step 7: Python Frontend (`python/econsim/`)

- `python/econsim/__init__.py` — import PySimulation from the native module
- `python/econsim/config.py` — `StrategyConfig` and `SimConfig` dataclasses with `to_json()` method (spec page 12)
- `python/econsim/runner.py` — `run_simulation(config, n_ticks)` high-level entry point (spec page 13)

### Build & Verify

```bash
# Build Rust + Python module
maturin develop --release

# Verify from Python
python -c "
from econsim.runner import run_simulation
results = run_simulation(n_ticks=5000)
print(f'Final price: {results[\"prices\"][-1]:.2f}')
print(f'Price std: {results[\"prices\"].std():.4f}')
print(f'Total volume: {results[\"volumes\"].sum():.0f}')
"
```

## Reference

- `DESIGN_CHOICES.md` — detailed analysis of architectural tradeoffs (memory layout, sort strategies, state ownership, precision, etc.)
- `econ_sim_claude_code_prompt.md.pdf` — full original spec with all struct definitions and code snippets
- GPU is accessible: GTX 1080 Ti, CUDA 12.4, driver 580.126.09. Verified working with `nvidia-smi`.
