// Minimal library interface for vizza
// This allows examples and tests to import common types

pub mod app;
pub mod config;
pub mod depth_renderer;
pub mod depth_snapshot;
pub mod depth_timeline;
pub mod depth_timeline_renderer;
pub mod depth_timeline_window;
pub mod depth_window;
pub mod event;
pub mod live;
pub mod live_view;
pub mod loader;
pub mod plot_builder;
pub mod price_spacing;
pub mod renderer;
pub mod state;
pub mod time_spacing;
pub mod view;
pub mod zoom;

// Re-export commonly used types
pub use zoom::{LodLevel, ZoomX};
// pub use date_filter::{CalendarMap, Rules, Span, build_calendar_map_range};
pub use config::{ColorPalette, Config, Theme, ViewSettings};
pub use event::MouseState;
pub use loader::{MarketData, load_market_data};
pub use price_spacing::{PriceTickSpacing, select_price_spacing};
pub use state::{LineOverlay, PositionOverlay, PriceLevelQuad, ViewportState};
pub use time_spacing::{TimeTickSpacing, TimeUnit, candidate_steps, select_time_spacing};

// Re-export public API for creating plots
pub use app::{run_with_config, run_with_config_and_source, run_with_config_and_sources};
pub use plot_builder::{PlotBuilder, plot, plot_with_config};

// Re-export depth types
pub use depth_renderer::DepthRenderer;
pub use depth_snapshot::DepthSnapshot;
pub use depth_timeline::{DepthTimeline, DepthTimelineEntry, DepthTimelineState};
pub use depth_timeline_renderer::DepthTimelineRenderer;
pub use depth_timeline_window::DepthTimelineWindow;
pub use depth_window::DepthWindow;

// Re-export live data sources
pub use live::{
    BackfillRequest, BackfillResponse, HistoricalBackfillSource, IPCLiveDataSource, LiveDataSource,
    MockBackfillSource,
};
