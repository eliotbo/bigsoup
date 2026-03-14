use super::{PositionOverlay, PriceLevelQuad};
use crate::{
    config::ViewSettings,
    loader::{DividendWithIndex, MarketData},
    zoom::{LodLevel, ZoomX},
};
use lod::LevelStore;
use std::sync::{Arc, Mutex};

/// Pure state for a viewport - no GPU resources
pub struct ViewportState {
    // Position and size
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    // Zoom and pan
    pub zoom: ZoomX,
    /// Unix timestamp (seconds) at the right edge of the viewport
    pub viewport_right_ts: i64,
    pub pan_offset_y: f32,

    // View settings
    pub view_settings: ViewSettings,

    // Y-axis range
    pub fixed_y_min: f32,
    pub fixed_y_max: f32,

    // Data references
    pub level_store: Arc<Mutex<LevelStore>>,
    pub data_start_utc: i64,
    pub data_end_utc: i64,

    // Dividends with precomputed indices
    pub dividends: Vec<DividendWithIndex>,

    // Ticker symbol for display
    pub ticker: Option<String>,

    // Optional title displayed in viewport
    pub title: Option<String>,

    // Highlighted position overlays (vertical time-based)
    pub position_overlays: Vec<PositionOverlay>,

    // Price level quads (horizontal price-based)
    pub price_level_quads: Vec<PriceLevelQuad>,

    /// Accumulated fractional pan (in seconds) not yet applied to viewport_right_ts
    pan_accumulator: f64,
}

impl ViewportState {
    /// Create a new ViewportState.
    ///
    /// # Arguments
    /// * `initial_left_ts` - Optional Unix timestamp (seconds) for the LEFT edge of the viewport.
    ///   If None, defaults to showing most recent data (right edge at data_end_utc).
    pub fn new(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        market_data: &MarketData,
        initial_left_ts: Option<i64>,
    ) -> Self {
        let (data_start_utc, data_end_utc) = market_data.time_range();

        let mut zoom = ZoomX::default();
        // Set the minimum LOD level based on data granularity
        zoom.set_min_lod_from_interval(market_data.min_interval_secs);
        // Default to the finest available interval so historical data is visible immediately
        zoom.current_lod_level = zoom.min_lod_level();

        // Calculate viewport_right_ts based on initial_left_ts
        // If no initial position specified, show most recent data (right edge at data_end)
        let viewport_right_ts = if let Some(left_ts) = initial_left_ts {
            // Estimate visible duration based on default bar width and LOD
            let bar_width = zoom.bar_width_px as f64;
            let gap = 1.0;
            let bar_spacing = bar_width + gap;
            let num_bars = (width as f64 / bar_spacing) as i64;
            let lod_seconds = zoom.current_lod_level.seconds() as i64;
            let visible_duration = num_bars * lod_seconds;

            // Right edge = left edge + visible duration
            left_ts + visible_duration
        } else {
            // Default: show most recent data at the right edge
            data_end_utc
        };

        Self {
            x,
            y,
            width,
            height,
            zoom,
            viewport_right_ts,
            pan_offset_y: 0.0,
            view_settings: ViewSettings::default(),
            fixed_y_min: 0.0,
            fixed_y_max: 100.0,
            level_store: Arc::clone(&market_data.level_store),
            data_start_utc,
            data_end_utc,
            dividends: market_data.dividends.clone(),
            ticker: None,
            title: None,
            position_overlays: Vec::new(),
            price_level_quads: Vec::new(),
            pan_accumulator: 0.0,
        }
    }

    /// Set the ticker symbol for this viewport
    pub fn with_ticker(mut self, ticker: String) -> Self {
        self.ticker = Some(ticker);
        self
    }

    /// Set the ticker symbol (mutable)
    pub fn set_ticker(&mut self, ticker: String) {
        self.ticker = Some(ticker);
    }

    /// Attach overlay spans to this viewport.
    pub fn with_position_overlays(mut self, overlays: Vec<PositionOverlay>) -> Self {
        self.position_overlays = overlays;
        self
    }

    /// Replace the current overlays.
    pub fn set_position_overlays(&mut self, overlays: Vec<PositionOverlay>) {
        self.position_overlays = overlays;
    }

    pub fn position_overlays(&self) -> &[PositionOverlay] {
        &self.position_overlays
    }

    /// Attach price level quads to this viewport.
    pub fn with_price_level_quads(mut self, quads: Vec<PriceLevelQuad>) -> Self {
        self.price_level_quads = quads;
        self
    }

    /// Replace the current price level quads.
    pub fn set_price_level_quads(&mut self, quads: Vec<PriceLevelQuad>) {
        self.price_level_quads = quads;
    }

    pub fn price_level_quads(&self) -> &[PriceLevelQuad] {
        &self.price_level_quads
    }

    pub fn with_title(mut self, title: String) -> Self {
        self.title = Some(title);
        self
    }

    pub fn set_title(&mut self, title: String) {
        self.title = Some(title);
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn get_num_bars_in_viewport(&self) -> u32 {
        self.zoom.get_num_bars_in_viewport(self.width as f64) as u32
    }

    /// Return the list of LOD levels that currently have candle data backing them
    pub fn lod_levels_with_data(&self) -> Vec<LodLevel> {
        let store = self
            .level_store
            .lock()
            .expect("level_store mutex poisoned");

        LodLevel::all_levels()
            .iter()
            .copied()
            .filter(|level| store.info(level.seconds() as u64).is_some())
            .collect()
    }

    /// Pan to the end of data with 1m LOD, positioned at 25% of viewport
    pub fn pan_to_end_with_lod(&mut self) {
        self.zoom.current_lod_level = LodLevel::M1;

        // Position the last data point at 25% of the viewport from the left
        // This means 75% empty space on the right for live data
        let num_bars = self.get_num_bars_in_viewport() as i64;
        let lod_seconds = self.zoom.current_lod_level.seconds() as i64;
        let visible_duration = num_bars * lod_seconds;

        // Use live data bounds so backfill data is included
        let (_, data_end) = self.current_data_bounds();

        // Right edge should be data_end + 75% of visible duration
        self.viewport_right_ts = data_end + (visible_duration * 3 / 4);

        // Reset accumulators
        self.pan_accumulator = 0.0;
        self.pan_offset_y = 0.0;
    }

    /// Calculate the valid pan range as absolute timestamps for viewport_right_ts
    /// Returns (min_right_ts, max_right_ts)
    pub fn calculate_pan_limits(&self) -> (i64, i64) {
        let lod_seconds = self.zoom.current_lod_level.seconds() as i64;
        let num_bars = self.get_num_bars_in_viewport() as i64;
        let visible_duration = num_bars * lod_seconds;

        // Query live data bounds from the level store (handles backfill + live appends)
        let (data_start, data_end) = self.current_data_bounds();

        // Min: can't scroll so far left that right edge is before data starts + visible width
        let min_right_ts = data_start + visible_duration;

        // Max: allow 75% empty space on right for live data
        let max_right_ts = data_end + (visible_duration * 3 / 4);

        // When data range is small relative to visible duration, min can exceed max
        if min_right_ts > max_right_ts {
            return (max_right_ts, min_right_ts);
        }

        (min_right_ts, max_right_ts)
    }

    /// Query current data time bounds from the level store.
    /// Returns (start_seconds, end_seconds) reflecting all appended data.
    fn current_data_bounds(&self) -> (i64, i64) {
        if let Ok(store) = self.level_store.lock() {
            let current_lod = self.zoom.current_lod_level.seconds() as u64;
            if let Some(info) = store.info(current_lod) {
                // LevelInfo timestamps are in nanoseconds, convert to seconds
                return (info.first_ts / 1_000_000_000, info.last_ts / 1_000_000_000);
            }
            // Fallback: check all intervals for the widest range
            let mut start = i64::MAX;
            let mut end = i64::MIN;
            for interval in store.intervals() {
                if let Some(info) = store.info(interval) {
                    start = start.min(info.first_ts / 1_000_000_000);
                    end = end.max(info.last_ts / 1_000_000_000);
                }
            }
            if start != i64::MAX && end != i64::MIN {
                return (start, end);
            }
        }
        // Final fallback to cached values
        (self.data_start_utc, self.data_end_utc)
    }

    /// Compute the effective seconds per visual bar based on actual data density.
    /// For uniformly-spaced data, this equals lod_seconds.
    /// For sparse data (with gaps), this will be larger, making panning match visual movement.
    fn compute_effective_seconds_per_bar(&self) -> f64 {
        let lod_seconds = self.zoom.current_lod_level.seconds() as f64;

        // Get the current visible candles to measure actual data density
        if let Some((candles, start_idx, end_idx)) = self.get_visible_candle_range() {
            let slice_len = end_idx - start_idx;
            if slice_len > 1 {
                let first_ts = candles[start_idx].ts;
                let last_ts = candles[end_idx - 1].ts;
                let time_span_secs = (last_ts - first_ts) as f64 / 1_000_000_000.0;
                let num_gaps = (slice_len - 1) as f64;

                if num_gaps > 0.0 && time_span_secs > 0.0 {
                    // Return the actual average time between candles
                    return time_span_secs / num_gaps;
                }
            }
        }

        // Fallback to theoretical LOD interval
        lod_seconds
    }

    /// Handle pan in X and Y directions
    pub fn handle_pan(&mut self, dx: f64, dy: f64, rescale_y: bool) {
        if rescale_y {
            // Y-axis rescaling mode: vertical motion rescales the y-axis
            let price_range = self.fixed_y_max - self.fixed_y_min;

            // Scale factor based on vertical motion
            // Drag down (dy > 0) expands the range (zoom out)
            // Drag up (dy < 0) contracts the range (zoom in)
            let scale_factor = 1.0 + (dy / self.height as f64) * 0.5;

            let new_range = price_range * scale_factor as f32;
            let center = (self.fixed_y_max + self.fixed_y_min) / 2.0;

            self.fixed_y_min = center - new_range / 2.0;
            self.fixed_y_max = center + new_range / 2.0;

            // Disable auto y-scale when manually rescaling
            self.view_settings.auto_y_scale = false;
        } else {
            // Normal panning mode
            // X panning
            let bar_width = self.zoom.bar_width_px as f64;
            let gap = 1.0;
            let bar_spacing = bar_width + gap;

            // Use effective seconds per bar based on actual data density
            // This ensures panning speed matches visual bar spacing even when data is sparse
            let seconds_per_bar = self.compute_effective_seconds_per_bar();

            // Convert pixel delta to time delta (keep as f64 for precision)
            // Drag right (dx > 0) shows earlier data → negative accumulator
            let bars_moved = dx / bar_spacing;
            let seconds_delta = bars_moved * seconds_per_bar;

            // Accumulate the fractional movement
            self.pan_accumulator -= seconds_delta;

            // Extract whole seconds to apply
            let whole_seconds = self.pan_accumulator.trunc() as i64;
            self.pan_accumulator -= whole_seconds as f64;

            self.viewport_right_ts += whole_seconds;

            // Apply pan limits based on actual data bounds
            let (min_ts, max_ts) = self.calculate_pan_limits();
            let clamped_ts = self.viewport_right_ts.clamp(min_ts, max_ts);

            // If we hit a pan limit, reset the accumulator to prevent "rubber band" effect
            // where dragging past the boundary builds up offset that must be unwound
            if clamped_ts != self.viewport_right_ts {
                self.pan_accumulator = 0.0;
            }
            self.viewport_right_ts = clamped_ts;

            // Y panning (only when auto_y_scale is false)
            if !self.view_settings.auto_y_scale {
                let price_range = self.fixed_y_max - self.fixed_y_min;
                let price_per_pixel = price_range / self.height as f32;

                // Drag down (dy > 0) shows higher prices (increase offset)
                self.pan_offset_y += dy as f32 * price_per_pixel;
            }
        }
    }

    /// Extract visible candles for current viewport and zoom level
    /// Returns (Arc of all candles, start_idx, end_idx) for the visible range
    pub fn get_visible_candle_range(&self) -> Option<(Arc<[lod::PlotCandle]>, usize, usize)> {
        // 1. Get the appropriate LOD interval
        let lod_seconds = self.zoom.current_lod_level.seconds() as i64;

        // 2. Get candles for this interval from level_store
        let candles = {
            let store = self.level_store.lock().expect("level_store mutex poisoned");
            store.get(lod_seconds as u64)
        }?;

        if candles.is_empty() {
            return Some((candles, 0, 0));
        }

        // 3. Calculate visible range using absolute timestamps
        let num_bars = self.get_num_bars_in_viewport() as i64;
        let visible_duration = num_bars * lod_seconds;
        let visible_start_ts = self.viewport_right_ts - visible_duration;

        // Convert seconds to nanoseconds for comparison with candle timestamps
        // (PlotCandle.ts is in nanoseconds, viewport_right_ts is in seconds)
        let visible_start_ns = visible_start_ts * 1_000_000_000;

        // Find start index based on visible_start_ts (time-based for correct left edge)
        let start_idx = match candles.binary_search_by(|c| c.ts.cmp(&visible_start_ns)) {
            Ok(i) => i,
            Err(i) => i, // Insert position is where we'd start
        };

        // Calculate end_idx based on candle COUNT, not time.
        // We need num_bars + 1 candles to fill positions -1 through num_bars - 1
        // (because first_bar_position = -1.0, so slice[0] is at position -1)
        // This handles market data with gaps (weekends, holidays) where the time span
        // of num_bars candles exceeds visible_duration.
        let needed_candles = num_bars as usize + 1;
        let end_idx = (start_idx + needed_candles).min(candles.len());

        // Clamp to valid bounds
        let start_idx = start_idx.min(candles.len());
        let (start_idx, end_idx) = if start_idx > end_idx {
            (end_idx, end_idx)
        } else {
            (start_idx, end_idx)
        };

        Some((candles, start_idx, end_idx))
    }

    /// Adjust viewport_right_ts so `target_ts` stays under the same horizontal bar index after an LOD change
    /// Note: target_ts is expected to be in nanoseconds (from PlotCandle.ts)
    pub fn align_timestamp_to_relative_index(
        &mut self,
        target_ts_ns: i64,
        desired_relative_index: usize,
    ) {
        let num_bars_in_viewport = self.get_num_bars_in_viewport() as i64;
        if num_bars_in_viewport == 0 || desired_relative_index >= num_bars_in_viewport as usize {
            return;
        }

        let lod_seconds = self.zoom.current_lod_level.seconds() as i64;

        // Convert nanosecond timestamp to seconds for our viewport positioning
        let target_ts = target_ts_ns / 1_000_000_000;

        // The target_ts should appear at desired_relative_index from the left edge
        // visible_start_ts + (desired_relative_index * lod_seconds) = target_ts
        // visible_start_ts = target_ts - (desired_relative_index * lod_seconds)
        // viewport_right_ts = visible_start_ts + (num_bars * lod_seconds)
        let visible_start_ts = target_ts - (desired_relative_index as i64 * lod_seconds);
        let new_right_ts = visible_start_ts + (num_bars_in_viewport * lod_seconds);

        // Clamp to valid bounds
        let (min_ts, max_ts) = self.calculate_pan_limits();
        self.viewport_right_ts = new_right_ts.clamp(min_ts, max_ts);

        // Reset pan accumulator when programmatically repositioning
        self.pan_accumulator = 0.0;
    }

    pub fn update_size(&mut self, x: f32, y: f32, width: f32, height: f32) {
        self.x = x;
        self.y = y;
        self.width = width;
        self.height = height;
    }

    pub fn update_y_range(&mut self, y_min: f32, y_max: f32) {
        if self.view_settings.auto_y_scale {
            self.fixed_y_min = y_min;
            self.fixed_y_max = y_max;
            self.pan_offset_y = 0.0; // Reset Y pan when auto-scaling
        }
    }
}
