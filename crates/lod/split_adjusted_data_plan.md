# Plan: Split-Corrected Data Support in `lod`

## Goal
Equip the `lod` crate with split-aware loaders so `.nohlcv`, `.ntrds`, and `.nbbo` readers, adapters, and level builders emit prices adjusted by companion `splits.json`, while preserving opt-in behavior and existing APIs for legacy consumers.

## Current Data Flow Audit
- `src/nohlcv_decoder.rs` (`NohlcvReader`, `RecordIterator`, `OhlcvRecord::has_valid_prices`) feeds `viz_adapter::load_nohlcv_data`, `loader::load_file_into_store`, and `StreamingAggregator`. Prices live in nanodollar `i64`s with `NULL_PRICE = i64::MAX` sentinel.
- `src/ntrd_decoder.rs` exposes `TradeRecord` with `price: i64`, optional `RecordIterator`, and implements `QuoteLike` near line ~900. `PlotTrade` conversion and statistics utilities assume raw prices.
- `src/nbbo_decoder.rs` (`NbboReader`, `NbboRecord`) stores bid/ask in optional `i64`, using `-1` as null sentinel; mid-price calculations flow into `traits::QuoteLike for NbboRecord`.
- `src/loader.rs` constructs `PlotCandle`s from `NohlcvReader` output and caches metadata; `.nbbo` and `.ntrds` paths are stubbed or skipped but should share adjustment logic once enabled.
- `src/viz_adapter.rs` (`load_nohlcv_data`, `load_nohlcv_level_store_with_days`, `aggregate_by_interval`) depends directly on raw `NohlcvReader` values and is exercised by `tests/test_viz_adapter.rs`.
- Memory-mapped readers in `src/nohlcv_mmap.rs`, `src/ntrd_mmap.rs`, and `src/nbbo_mmap.rs` mirror decoder structs (`*_RecordZC`) and must mirror any adjustment path to stay consistent with feature flags.
- Integration tests in `tests/nohlcv_test.rs`, `tests/test_loader.rs`, and `tests/real_data_test.rs` glue readers into aggregators; these will validate adjusted outputs against the FAST fixture at `../../../../data/consolidated/stock-split-dividend-test/FAST/`.

## Implementation Roadmap
- **Shared Split Context (new module)**: Add `src/splits.rs` exposing `StockSplit`, `SplitCalendar`, and helpers to load multi-ticker JSON (reuse schema from `stock_split_adjustment_guide.md`). Provide `SplitAdjuster::factor_for(timestamp_ns)` that caches cumulative ratios per trading session (date derived via `chrono::NaiveDate`).
- **NOHLCV Integration**: Extend `NohlcvReader` to accept optional `SplitAdjuster` (via `with_split_adjuster` builder or `RecordIterator::with_adjuster`). Adjust raw `i64` fields before exposure while preserving `NULL_PRICE` semantics. Rounding should be centralized (e.g., `scale::adjust_price(value: i64, factor: f64) -> i64`) to maintain nanodollar precision and avoid f64::MAX overflow. Mirror logic inside `nohlcv_mmap::NohlcvMmapReader::get_record`.
- **Trade Stream Integration**: Introduce optional adjustment in `ntrd_decoder::NtrdReader::read_record` and iterators so `TradeRecord::price` reflects split-adjusted values. Ensure `TradeRecord::to_bytes`/`from_bytes` remain raw, but public iteration defaults to adjusted when configured. Update `ntrd_mmap::NtrdMmapReader` to offer the same adjustment hook. Maintain odd-lot and flag handling by adjusting only price field.
- **NBBO Integration**: Modify `nbbo_decoder::NbboReader` to adjust `bid_px`/`ask_px` (and derived mid-price) via the same helper, skipping `None` levels. Propagate to `nbbo_mmap::NbboRecordZC::to_record`. Consider depth snapshots if future support extends beyond top-of-book.
- **Configuration Surface**: Define `SplitAdjustmentConfig` (e.g., `lod::loader::SplitPolicy`) capturing `adjust: bool`, `splits_path: Option<PathBuf>`, and maybe `ticker_override`. Thread config through `DataLoader::load_file_into_store`, `viz_adapter::load_data_unified`, `StreamingAggregator` helpers, and CLI binaries under `src/bin/` (notably `nohlcv_decoder.rs`, `ntrd_decoder.rs`). Default to off to keep backward compatibility; allow auto-discovery of `splits.json` alongside data file when config is `Auto`.
- **Discovery & Binding**: Implement directory scan in `loader::scan_symbol_directory` (or adjacent helper) to associate `.nohlcv` / `.ntrds` / `.nbbo` with a sibling `splits.json`. Cache parsed splits per symbol to avoid repeated I/O. Provide manual override via API for callers already holding split metadata.
- **Downstream Consumers**: Update `viz_adapter::load_nohlcv_data` and `load_nohlcv_level_store_with_days` to accept config or detect splits automatically, ensuring `PlotCandle` values are adjusted before aggregation. Extend `StreamingAggregator::push` call sites to operate on adjusted `QuoteLike` wrappers without altering aggregator internals.
- **Testing & Validation**: Add unit tests in a new `tests/splits_test.rs` covering cumulative factor math, including forward and reverse splits. Extend `tests/nohlcv_test.rs` and `tests/test_loader.rs` to load the FAST fixture twice (adjusted vs raw) and assert continuity across split boundaries (`pre_split_close * ratio ≈ post_split_open`). Add smoke tests for `.ntrds` and `.nbbo` once sample files exist, stubbing with synthetic records if necessary. Ensure gzip/feature-flagged mmap tests (`tests/mmap_test.rs`) run with adjustments enabled.
- **Documentation**: Update `README.md` and `stock_split_adjustment_guide.md` to point users at the new config and describe default behavior (adjusted off by default, opt-in recommended). Add doc comments in `splits.rs` and module-level docs in `nohlcv_decoder.rs`/`ntrd_decoder.rs` noting the availability of split correction.

## Open Questions / Follow-Ups
- Should adjusted and raw streams be accessible simultaneously (e.g., via new iterator that yields both) or should we expose separate constructors? This affects `traits::OhlcvRecord` cloning strategy.
- Is it preferable to depend on `db3` for split extraction instead of duplicating the JSON loader in `lod`? Evaluate feature flags or shared crate usage.
- How do we source `.ntrds`/`.nbbo` fixtures with real splits (FAST only covers `.nohlcv`)? Consider generating synthetic data for regression tests.
- Do we need per-session caching to avoid recomputing factors for every timestamp, or is a simple `BTreeMap<Date, f64>` adequate given typical file sizes?

## Acceptance Criteria
- Loading the FAST fixture through `viz_adapter::load_nohlcv_data` with adjustments produces continuity across the split date; disabling adjustments restores raw gaps.
- `DataLoader` surfaces adjusted `PlotCandle`s, and trade/quote iterations show corrected prices when config is enabled.
- Memory-mapped readers remain binary-compatible yet return adjusted values when the same config is applied.
- All updated tests pass with and without the split-adjustment feature flag, and no regressions appear in existing consumers.
