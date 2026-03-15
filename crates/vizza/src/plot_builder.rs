use crate::{Config, LodLevel, LineOverlay, MarketData, PositionOverlay, PriceLevelQuad, Theme, ViewSettings};
use anyhow::Result;
use lod::LevelStore;

/// Builder for creating and running vizza plots
pub struct PlotBuilder {
    data_paths: Vec<String>,
    in_memory_market_data: Option<MarketData>,
    per_viewport_market_data: Option<Vec<MarketData>>,
    window_width: u32,
    window_height: u32,
    grid_rows: usize,
    grid_cols: usize,
    default_lod_level: LodLevel,
    view_settings: ViewSettings,
    use_live_data: bool,
    live_update_interval_ms: Option<u64>,
    allow_missing_history: bool,
    custom_live_source: Option<Box<dyn crate::live::LiveDataSource>>,
    custom_live_sources: Vec<(String, Box<dyn crate::live::LiveDataSource>)>,
    today_so_far_enabled: bool,
    position_overlays: Option<Vec<Vec<PositionOverlay>>>,
    price_level_quads: Option<Vec<Vec<PriceLevelQuad>>>,
    line_overlays: Option<Vec<Vec<LineOverlay>>>,
    tickers: Option<Vec<Option<String>>>,
    titles: Option<Vec<Option<String>>>,
    single_title: Option<String>,
    bar_width_px: u16,
    /// Initial left edge timestamp (Unix seconds) for all viewports
    initial_left_ts: Option<i64>,
    /// Per-viewport initial left edge timestamps (Unix seconds)
    initial_left_times: Option<Vec<Option<i64>>>,
    theme: Theme,
}

impl PlotBuilder {
    /// Create a new PlotBuilder with default settings
    pub fn new() -> Self {
        let config = Config::default();
        Self {
            data_paths: config.data_paths,
            in_memory_market_data: None,
            per_viewport_market_data: None,
            window_width: config.window_width,
            window_height: config.window_height,
            grid_rows: config.grid_rows,
            grid_cols: config.grid_cols,
            default_lod_level: config.default_lod_level,
            view_settings: config.view_settings,
            use_live_data: false,
            live_update_interval_ms: None,
            allow_missing_history: config.allow_missing_history,
            custom_live_source: None,
            custom_live_sources: Vec::new(),
            today_so_far_enabled: false,
            position_overlays: None,
            price_level_quads: None,
            line_overlays: None,
            tickers: None,
            titles: None,
            single_title: None,
            bar_width_px: config.bar_width_px,
            initial_left_ts: None,
            initial_left_times: None,
            theme: config.theme,
        }
    }

    /// Set the data paths for each viewport
    pub fn with_data_paths(mut self, paths: Vec<String>) -> Self {
        self.data_paths = paths;
        self.in_memory_market_data = None;
        self.per_viewport_market_data = None;
        self
    }

    /// Set the window size
    pub fn with_window_size(mut self, width: u32, height: u32) -> Self {
        self.window_width = width;
        self.window_height = height;
        self
    }

    /// Provide an in-memory level store instead of loading from disk
    ///
    /// This clears any previously configured data paths.
    pub fn with_level_store(mut self, level_store: LevelStore) -> Self {
        self.in_memory_market_data = Some(MarketData::from_level_store(level_store));
        self.data_paths.clear();
        self.per_viewport_market_data = None;
        self
    }

    /// Provide a fully constructed MarketData object for rendering.
    ///
    /// This clears any previously configured data paths.
    pub fn with_market_data(mut self, market_data: MarketData) -> Self {
        self.in_memory_market_data = Some(market_data);
        self.data_paths.clear();
        self.per_viewport_market_data = None;
        self
    }

    /// Provide per-viewport MarketData instances.
    ///
    /// The number of entries must match the configured grid size (rows * cols).
    /// This clears any previously configured data paths or shared market data.
    pub fn with_market_data_views(mut self, market_data: Vec<MarketData>) -> Self {
        self.per_viewport_market_data = Some(market_data);
        self.in_memory_market_data = None;
        self.data_paths.clear();
        self
    }

    /// Set the grid dimensions (rows x cols)
    pub fn with_grid(mut self, rows: usize, cols: usize) -> Self {
        self.grid_rows = rows;
        self.grid_cols = cols;
        self
    }

    /// Set the default LOD (Level of Detail) level
    pub fn with_lod_level(mut self, level: LodLevel) -> Self {
        self.default_lod_level = level;
        self
    }

    /// Enable or disable live data updates
    pub fn with_live_data(mut self, enabled: bool, update_interval_ms: Option<u64>) -> Self {
        self.use_live_data = enabled;
        self.live_update_interval_ms = update_interval_ms;
        self
    }

    /// Enable or disable automatic Y-axis scaling
    pub fn with_auto_y_scale(mut self, enabled: bool) -> Self {
        self.view_settings.auto_y_scale = enabled;
        self
    }

    /// Enable or disable volume bar overlay rendering.
    pub fn with_volume_bars(mut self, enabled: bool) -> Self {
        self.view_settings.show_volume_bars = enabled;
        self
    }

    /// Allow chart to run without historical data files
    pub fn with_allow_missing_history(mut self, enabled: bool) -> Self {
        self.allow_missing_history = enabled;
        self
    }

    /// Set the initial bar width (must be 1, 3, 5, or 7 pixels).
    pub fn with_bar_width_px(mut self, bar_width_px: u16) -> Self {
        match bar_width_px {
            1 | 3 | 5 | 7 => self.bar_width_px = bar_width_px,
            _ => panic!(
                "Unsupported bar width: {}. Allowed values are 1, 3, 5, or 7.",
                bar_width_px
            ),
        }
        self
    }

    /// Set a custom live data source (for single viewport or backward compatibility)
    pub fn with_custom_live_source(mut self, source: Box<dyn crate::live::LiveDataSource>) -> Self {
        self.custom_live_source = Some(source);
        self
    }

    /// Set multiple custom live data sources, one per viewport
    ///
    /// Each source is paired with its ticker symbol. The number of sources should match
    /// grid_rows * grid_cols. If fewer sources are provided, remaining viewports will
    /// use default data sources.
    ///
    /// # Example
    /// ```no_run
    /// use vizza::PlotBuilder;
    ///
    /// let sources = vec![
    ///     ("AAPL".to_string(), Box::new(AppleDataSource::new())),
    ///     ("MSFT".to_string(), Box::new(MsftDataSource::new())),
    /// ];
    ///
    /// PlotBuilder::new()
    ///     .with_grid(1, 2)
    ///     .with_custom_live_sources(sources)
    ///     .with_live_data(true, Some(100))
    ///     .run();
    /// ```
    pub fn with_custom_live_sources(
        mut self,
        sources: Vec<(String, Box<dyn crate::live::LiveDataSource>)>,
    ) -> Self {
        self.custom_live_sources = sources;
        self
    }

    /// Enable "today-so-far" historical backfill.
    ///
    /// When enabled, vizza will attempt to fetch intraday bars from market open
    /// to the current time before starting live trading. This requires a data source
    /// that implements `HistoricalBackfillSource`.
    ///
    /// # Example
    /// ```no_run
    /// use vizza::PlotBuilder;
    ///
    /// PlotBuilder::new()
    ///     .with_today_so_far(true)
    ///     .with_live_data(true, Some(100))
    ///     .run();
    /// ```
    pub fn with_today_so_far(mut self, enabled: bool) -> Self {
        self.today_so_far_enabled = enabled;
        self
    }

    /// Provide per-viewport position overlay spans.
    ///
    /// The outer vector must match the grid size (rows * cols). Each inner
    /// vector holds the spans for a specific viewport, ordered in row-major
    /// order.
    pub fn with_position_overlays(mut self, overlays: Vec<Vec<PositionOverlay>>) -> Self {
        self.position_overlays = Some(overlays);
        self
    }

    /// Provide per-viewport price level quads (stop-loss and take-profit visualization).
    ///
    /// The outer vector must match the grid size (rows * cols). Each inner
    /// vector holds the price level quads for a specific viewport, ordered in row-major
    /// order.
    ///
    /// # Example
    /// ```no_run
    /// use vizza::{PlotBuilder, PriceLevelQuad};
    ///
    /// let quads = vec![
    ///     vec![
    ///         PriceLevelQuad::stop_loss(1234567890, 1234577890, 100.0, 95.0),
    ///         PriceLevelQuad::take_profit(1234567890, 1234577890, 100.0, 110.0),
    ///     ],
    ///     vec![],  // No quads for second viewport
    /// ];
    ///
    /// PlotBuilder::new()
    ///     .with_grid(1, 2)
    ///     .with_price_level_quads(quads)
    ///     .run();
    /// ```
    pub fn with_price_level_quads(mut self, quads: Vec<Vec<PriceLevelQuad>>) -> Self {
        self.price_level_quads = Some(quads);
        self
    }

    /// Provide per-viewport line overlays (e.g. EMA, SMA lines).
    ///
    /// The outer vector has one entry per viewport (row-major order). Each inner
    /// vector holds the line overlays for that viewport.
    pub fn with_line_overlays(mut self, overlays: Vec<Vec<LineOverlay>>) -> Self {
        self.line_overlays = Some(overlays);
        self
    }

    /// Assign per-viewport ticker strings.
    ///
    /// Provide one entry per viewport (row-major order). Use `None` for viewports
    /// that should omit the label.
    pub fn with_tickers(mut self, tickers: Vec<Option<String>>) -> Self {
        self.tickers = Some(tickers);
        self
    }

    /// Provide per-viewport titles rendered at the top-center of each viewport.
    ///
    /// Supply one entry per viewport (row-major order). Use `None` for viewports
    /// without a title.
    pub fn with_titles(mut self, titles: Vec<Option<String>>) -> Self {
        self.titles = Some(titles);
        self.single_title = None;
        self
    }

    /// Apply the same title to every viewport.
    pub fn with_title<T: Into<String>>(mut self, title: T) -> Self {
        self.single_title = Some(title.into());
        self.titles = None;
        self
    }

    /// Set the initial left edge of the viewport using a Unix timestamp (seconds).
    ///
    /// The chart will display data starting from this timestamp at the left edge.
    /// If None (default), the most recent data is shown at the right edge.
    ///
    /// # Example
    /// ```no_run
    /// use vizza::PlotBuilder;
    ///
    /// // Start at a specific date/time (Jan 15, 2025, 09:30 AM EST = 1736946600)
    /// PlotBuilder::new()
    ///     .with_start_time(1736946600)
    ///     .run();
    /// ```
    pub fn with_start_time(mut self, timestamp_secs: i64) -> Self {
        self.initial_left_ts = Some(timestamp_secs);
        self
    }

    /// Set per-viewport initial left edge positions using Unix timestamps (seconds).
    ///
    /// Takes one entry per viewport (row-major order). Use `None` for viewports
    /// that should show the most recent data (default behavior).
    /// `Some(timestamp_secs)` means the left edge starts at that Unix timestamp.
    ///
    /// When both `with_start_times` and `with_start_time` are used, per-viewport
    /// start times take precedence for viewports with `Some` values.
    ///
    /// # Example
    /// ```no_run
    /// use vizza::PlotBuilder;
    ///
    /// PlotBuilder::new()
    ///     .with_grid(2, 2)
    ///     .with_start_times(vec![
    ///         Some(1699900000),  // viewport 0 starts here
    ///         Some(1700000000),  // viewport 1 starts here
    ///         None,              // viewport 2 uses default (most recent)
    ///         Some(1700100000),  // viewport 3 starts here
    ///     ])
    ///     .run();
    /// ```
    pub fn with_start_times(mut self, timestamps: Vec<Option<i64>>) -> Self {
        self.initial_left_times = Some(timestamps);
        self
    }

    /// Set the color theme for the visualization.
    ///
    /// # Example
    /// ```no_run
    /// use vizza::{PlotBuilder, Theme};
    ///
    /// PlotBuilder::new()
    ///     .with_theme(Theme::Dark)
    ///     .run();
    /// ```
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Run the plot application with the configured settings
    pub fn run(self) -> Result<()> {
        let total_viewports = self.grid_rows * self.grid_cols;

        // Pad position overlays to match total viewports
        let mut position_overlays = self.position_overlays.unwrap_or_default();
        if !position_overlays.is_empty() && position_overlays.len() > total_viewports {
            return Err(anyhow::anyhow!(
                "Too many position overlay lists: expected at most {} for the {}x{} grid, received {}",
                total_viewports,
                self.grid_rows,
                self.grid_cols,
                position_overlays.len()
            ));
        }
        while position_overlays.len() < total_viewports {
            position_overlays.push(Vec::new());
        }

        // Pad price level quads to match total viewports
        let mut price_level_quads = self.price_level_quads.unwrap_or_default();
        if !price_level_quads.is_empty() && price_level_quads.len() > total_viewports {
            return Err(anyhow::anyhow!(
                "Too many price level quad lists: expected at most {} for the {}x{} grid, received {}",
                total_viewports,
                self.grid_rows,
                self.grid_cols,
                price_level_quads.len()
            ));
        }
        while price_level_quads.len() < total_viewports {
            price_level_quads.push(Vec::new());
        }

        // Pad line overlays to match total viewports
        let mut line_overlays = self.line_overlays.unwrap_or_default();
        if !line_overlays.is_empty() && line_overlays.len() > total_viewports {
            return Err(anyhow::anyhow!(
                "Too many line overlay lists: expected at most {} for the {}x{} grid, received {}",
                total_viewports,
                self.grid_rows,
                self.grid_cols,
                line_overlays.len()
            ));
        }
        while line_overlays.len() < total_viewports {
            line_overlays.push(Vec::new());
        }

        // Pad tickers to match total viewports
        let mut tickers = self.tickers.unwrap_or_default();
        if !tickers.is_empty() && tickers.len() > total_viewports {
            return Err(anyhow::anyhow!(
                "Too many ticker entries: expected at most {} for the {}x{} grid, received {}",
                total_viewports,
                self.grid_rows,
                self.grid_cols,
                tickers.len()
            ));
        }
        while tickers.len() < total_viewports {
            tickers.push(None);
        }

        // Pad titles to match total viewports
        let mut titles = if let Some(titles) = self.titles {
            titles
        } else if let Some(title) = self.single_title {
            vec![Some(title); total_viewports]
        } else {
            Vec::new()
        };

        if !titles.is_empty() && titles.len() > total_viewports {
            return Err(anyhow::anyhow!(
                "Too many title entries: expected at most {} for the {}x{} grid, received {}",
                total_viewports,
                self.grid_rows,
                self.grid_cols,
                titles.len()
            ));
        }
        while titles.len() < total_viewports {
            titles.push(None);
        }

        // Pad initial_left_times to match total viewports
        let mut initial_left_times = self.initial_left_times.unwrap_or_default();
        if !initial_left_times.is_empty() && initial_left_times.len() > total_viewports {
            return Err(anyhow::anyhow!(
                "Too many initial_left_times entries: expected at most {} for the {}x{} grid, received {}",
                total_viewports,
                self.grid_rows,
                self.grid_cols,
                initial_left_times.len()
            ));
        }
        while initial_left_times.len() < total_viewports {
            initial_left_times.push(None);
        }

        let config = Config {
            data_paths: self.data_paths,
            window_width: self.window_width,
            window_height: self.window_height,
            grid_rows: self.grid_rows,
            grid_cols: self.grid_cols,
            default_lod_level: self.default_lod_level,
            view_settings: self.view_settings,
            allow_missing_history: self.allow_missing_history,
            position_overlays,
            price_level_quads,
            line_overlays,
            tickers,
            titles,
            bar_width_px: self.bar_width_px,
            initial_left_ts: self.initial_left_ts,
            initial_left_times,
            theme: self.theme,
        };

        // Pad per-viewport market data to match total viewports
        let per_viewport_market_data = if let Some(mut datasets) = self.per_viewport_market_data {
            let expected = self.grid_rows * self.grid_cols;
            if datasets.len() > expected {
                return Err(anyhow::anyhow!(
                    "Too many in-memory datasets: expected at most {} for the {}x{} grid, received {}",
                    expected,
                    self.grid_rows,
                    self.grid_cols,
                    datasets.len()
                ));
            }
            // Pad with empty MarketData instances
            while datasets.len() < expected {
                datasets.push(MarketData::from_level_store(LevelStore::new()));
            }
            Some(datasets)
        } else {
            None
        };
        let in_memory_market_data = self.in_memory_market_data;
        let use_live_data = self.use_live_data;
        let live_update_interval_ms = self.live_update_interval_ms;
        let today_so_far_enabled = self.today_so_far_enabled;
        let custom_live_sources = self.custom_live_sources;
        let custom_live_source = self.custom_live_source;
        if !custom_live_sources.is_empty() {
            // Multiple sources path
            crate::run_with_config_and_sources(
                config,
                use_live_data,
                live_update_interval_ms,
                custom_live_sources,
                today_so_far_enabled,
                in_memory_market_data,
                per_viewport_market_data,
            )
        } else if custom_live_source.is_some() {
            // Single source path (backward compatibility)
            crate::run_with_config_and_source(
                config,
                use_live_data,
                live_update_interval_ms,
                custom_live_source,
                today_so_far_enabled,
                in_memory_market_data,
                per_viewport_market_data,
            )
        } else {
            crate::run_with_config(
                config,
                use_live_data,
                live_update_interval_ms,
                in_memory_market_data,
                per_viewport_market_data,
            )
        }
    }
}

impl Default for PlotBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple function to create a plot with basic parameters
pub fn plot(
    data_paths: Vec<String>,
    grid_rows: usize,
    grid_cols: usize,
    use_live_data: bool,
) -> Result<()> {
    PlotBuilder::new()
        .with_data_paths(data_paths)
        .with_grid(grid_rows, grid_cols)
        .with_live_data(use_live_data, None)
        .run()
}

/// Create a plot with a full Config object
pub fn plot_with_config(
    config: Config,
    use_live_data: bool,
    live_update_interval_ms: Option<u64>,
) -> Result<()> {
    crate::run_with_config(config, use_live_data, live_update_interval_ms, None, None)
}
