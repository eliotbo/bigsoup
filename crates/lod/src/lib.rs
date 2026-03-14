//! Level-of-detail (LOD) crate for high-performance multi-resolution data pipelines
//!
//! This crate provides streaming, multi-interval aggregation with incremental updates
//! and optional parallel cold starts for candles, NBBO, and other market data.

pub mod aggregator;
pub mod levels;
pub mod line_lod;
pub mod live;
pub mod loader;
pub mod nbbo_decoder;
pub mod nohlcv_decoder;
pub mod ntrd_decoder;
pub mod splits;
pub mod traits;
pub mod viz_adapter;
pub mod window;

#[cfg(feature = "bench")]
pub mod bench;

#[cfg(any(feature = "mmap", feature = "zerocopy"))]
pub mod mmap;

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub mod nbbo_mmap;

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub mod nohlcv_mmap;

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub mod ntrd_mmap;

pub use aggregator::StreamingAggregator;
pub use levels::{LevelInfo, LevelStore, PlotCandle, PlotData, PlotTrade};
pub use line_lod::{AggregatedPoint, MultiResolutionSeries, ProgressiveLodLoader, ResolutionLevel};
pub use live::{
    BarAgg, HotTradesRing, LiveEngine, LiveMetrics, LiveSnapshot, LodLane, MultiStreamSimulator,
    TradeSimulator,
};
pub use loader::{DataFile, DataLoader, FileType, SymbolMetadata};
pub use splits::{
    adjust_price_by_factor, discover_and_load_dividends, discover_and_load_splits,
    load_all_splits_from_json, load_dividends_divbin, load_splits_from_json, DividendEvent,
    DividendType, MultiTickerSplitsData, SplitAdjuster, SplitCalendar, StockSplit, NULL_PRICE,
};
pub use traits::{LevelGenerator, QuoteLike};
pub use viz_adapter::{
    aggregate_by_interval, format_detection, format_timestamp, import_candles, load_data_unified,
    load_nohlcv_data, load_nohlcv_level_store, load_nohlcv_level_store_with_days, trading_time,
    view_utils, Candle, DataSource, Metadata, Ohlc, Time,
};
pub use window::{take_window, take_window_duration, ChunkRef};

#[cfg(any(feature = "mmap", feature = "zerocopy"))]
pub use mmap::{MmapError, MmappedArray, MmappedFile};
