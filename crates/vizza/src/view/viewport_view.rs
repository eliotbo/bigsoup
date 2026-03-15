use super::{
    bars::{BarRenderer, MarkerInstanceData},
    grid::GridRenderer,
    line_overlay::{LineOverlayRenderer, LineVertex},
    position_overlay::{PositionOverlayInstanceData, PositionOverlayRenderer},
    price_level_quad::{PriceLevelQuadInstanceData, PriceLevelQuadRenderer},
};
use crate::{
    live::LiveDataSource,
    live_view::{LiveDataManager, LiveRenderData, LiveSnapshotState, PriceTrend},
    state::ViewportState,
};
use lod::PlotCandle;
use std::sync::{Arc, Mutex};

/// Left padding in bars to ensure the first bar and its wick are fully visible
const LEFT_PADDING_BARS: f32 = 1.0;

/// Holds GPU renderers and live data manager for a viewport
pub struct ViewportView {
    pub bars: BarRenderer,
    pub grid: GridRenderer,
    pub position_overlays: PositionOverlayRenderer,
    pub price_level_quads: PriceLevelQuadRenderer,
    pub line_overlays: LineOverlayRenderer,
    pub live_manager: Option<LiveDataManager>,
    cached_live_data: Option<LiveRenderData>,
    live_overlay_cache: Vec<PlotCandle>,
    last_lod_warning: Option<crate::zoom::LodLevel>, // Track last LOD that triggered a warning
}

impl ViewportView {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        state: &ViewportState,
        window_w: f32,
        window_h: f32,
        level_store: Arc<Mutex<lod::LevelStore>>,
        enable_live_pipeline: bool,
        palette: &crate::config::ColorPalette,
    ) -> Self {
        let mut bars = BarRenderer::new(
            device,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
            palette.candle_up_market,
            palette.candle_up_offhours,
            palette.candle_down_market,
            palette.candle_down_offhours,
            palette.wick,
            palette.volume,
        );
        bars.set_volume_enabled(state.view_settings.show_volume_bars);

        let mut position_overlays = PositionOverlayRenderer::new(
            device,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
        );
        position_overlays.update_view_uniform(&bars.view_uniform, queue);

        let mut price_level_quads = PriceLevelQuadRenderer::new(
            device,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
        );
        price_level_quads.update_view_uniform(&bars.view_uniform, queue);

        let mut line_overlays = LineOverlayRenderer::new(
            device,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
        );
        line_overlays.update_view_uniform(&bars.view_uniform, queue);

        let grid = GridRenderer::new(
            device,
            queue,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
            palette.grid_line,
            palette.text_primary,
            palette.text_secondary,
        );

        let live_manager = if enable_live_pipeline {
            Some(LiveDataManager::new(level_store))
        } else {
            None
        };

        Self {
            bars,
            grid,
            position_overlays,
            price_level_quads,
            line_overlays,
            live_manager,
            cached_live_data: None,
            live_overlay_cache: Vec::new(),
            last_lod_warning: None,
        }
    }

    /// Create a new ViewportView with a custom live data source
    pub fn new_with_custom_source(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        state: &ViewportState,
        window_w: f32,
        window_h: f32,
        level_store: Arc<Mutex<lod::LevelStore>>,
        custom_source: Option<Box<dyn LiveDataSource>>,
        ticker: &str,
        today_so_far_enabled: bool,
        palette: &crate::config::ColorPalette,
    ) -> Self {
        let mut bars = BarRenderer::new(
            device,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
            palette.candle_up_market,
            palette.candle_up_offhours,
            palette.candle_down_market,
            palette.candle_down_offhours,
            palette.wick,
            palette.volume,
        );
        bars.set_volume_enabled(state.view_settings.show_volume_bars);

        let mut position_overlays = PositionOverlayRenderer::new(
            device,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
        );
        position_overlays.update_view_uniform(&bars.view_uniform, queue);

        let mut price_level_quads = PriceLevelQuadRenderer::new(
            device,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
        );
        price_level_quads.update_view_uniform(&bars.view_uniform, queue);

        let mut line_overlays = LineOverlayRenderer::new(
            device,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
        );
        line_overlays.update_view_uniform(&bars.view_uniform, queue);

        let grid = GridRenderer::new(
            device,
            queue,
            format,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
            palette.grid_line,
            palette.text_primary,
            palette.text_secondary,
        );

        let live_manager = if let Some(source) = custom_source {
            let mut manager = LiveDataManager::with_data_source_and_options(
                Arc::clone(&level_store),
                source,
                ticker,
                today_so_far_enabled,
            );

            // Initialize backfill if enabled
            if today_so_far_enabled {
                if let Err(e) = manager.initialize_with_backfill() {
                    eprintln!("⚠ Failed to initialize backfill: {}", e);
                }
            }

            Some(manager)
        } else {
            Some(LiveDataManager::new(Arc::clone(&level_store)))
        };

        Self {
            bars,
            grid,
            position_overlays,
            price_level_quads,
            line_overlays,
            live_manager,
            cached_live_data: None,
            live_overlay_cache: Vec::new(),
            last_lod_warning: None,
        }
    }

    /// Update view based on current state
    /// Returns (y_min, y_max) of visible data for state updates
    pub fn update(&mut self, state: &ViewportState, queue: &wgpu::Queue) -> Option<(f32, f32)> {
        let update_start = std::time::Instant::now();

        // Update bar width from state
        self.bars.view_uniform.window[2] = state.zoom.bar_width_px as f32;
        self.bars
            .set_volume_enabled(state.view_settings.show_volume_bars);
        self.bars.update_view_uniform(queue);
        self.position_overlays
            .update_view_uniform(&self.bars.view_uniform, queue);
        self.price_level_quads
            .update_view_uniform(&self.bars.view_uniform, queue);
        self.line_overlays
            .update_view_uniform(&self.bars.view_uniform, queue);

        let lod_level = state.zoom.current_lod_level;
        let interval_secs = lod_level.seconds() as u64;
        let num_bars_in_viewport = state.get_num_bars_in_viewport();
        let min_lod_level = state.zoom.min_lod_level();

        // Check if current LOD is finer than what historical data supports
        let skip_historical_data = {
            let all_levels = crate::zoom::LodLevel::all_levels();
            let current_idx = all_levels.iter().position(|&l| l == lod_level).unwrap_or(0);
            let min_idx = all_levels
                .iter()
                .position(|&l| l == min_lod_level)
                .unwrap_or(0);
            current_idx < min_idx
        };

        // Issue warning once per LOD level when accessing unsupported granularity
        if skip_historical_data && self.last_lod_warning != Some(lod_level) {
            println!(
                "⚠️  LOD {} is finer than historical data granularity ({}). \
                Showing live data only at this zoom level.",
                lod_level.label(),
                min_lod_level.label()
            );
            self.last_lod_warning = Some(lod_level);
        } else if !skip_historical_data && self.last_lod_warning.is_some() {
            // Reset warning when returning to supported LOD
            self.last_lod_warning = None;
        }

        // Historical slice retains ownership via this holder
        let mut _hist_arc: Option<Arc<[PlotCandle]>> = None;
        let mut historical_slice: &[PlotCandle] = &[];
        let mut visible_start_idx: usize = 0;

        if !skip_historical_data {
            if let Some((candles, start_idx, end_idx)) = state.get_visible_candle_range() {
                _hist_arc = Some(candles);
                visible_start_idx = start_idx;
                if let Some(arc_ref) = _hist_arc.as_ref() {
                    historical_slice = &arc_ref[start_idx..end_idx];
                }
            }
        }

        // Pull current live data snapshot (cached if unchanged)
        let active_live_data = if let Some(manager) = self.live_manager.as_mut() {
            match manager.prepare_render(interval_secs) {
                LiveSnapshotState::Updated(data) => {
                    self.cached_live_data = Some(data);
                    self.cached_live_data.as_ref()
                }
                LiveSnapshotState::Unchanged => self.cached_live_data.as_ref(),
                LiveSnapshotState::Unavailable => {
                    self.cached_live_data = None;
                    None
                }
            }
        } else {
            self.cached_live_data = None;
            None
        };

        self.live_overlay_cache.clear();
        if let Some(data) = active_live_data {
            self.live_overlay_cache
                .extend(data.completed.iter().copied());
            if let Some(open) = data.open_bar {
                self.live_overlay_cache.push(open);
            }
        }

        if historical_slice.is_empty() && self.live_overlay_cache.is_empty() {
            self.refresh_position_overlays(
                queue,
                state,
                lod_level,
                num_bars_in_viewport,
                historical_slice,
            );
            self.bars.update_instances_with_range(
                &[],
                lod_level,
                queue,
                state.fixed_y_min,
                state.fixed_y_max,
                num_bars_in_viewport,
                -LEFT_PADDING_BARS,
                0.0,
            );
            self.bars.clear_live_instances();
            self.bars.update_marker_instance(None, queue);
            self.grid.update_grid_lines(
                queue,
                &[],
                lod_level,
                state.fixed_y_min,
                state.fixed_y_max,
                num_bars_in_viewport,
                &state.dividends,
                0,
                state.ticker.as_deref(),
                state.title(),
            );
            return None;
        }

        let mut observed_min = f32::MAX;
        let mut observed_max = f32::MIN;
        let mut max_volume = 0.0_f32;

        for candle in historical_slice {
            observed_min = observed_min.min(candle.low);
            observed_max = observed_max.max(candle.high);
            max_volume = max_volume.max(candle.volume.max(0.0));
        }
        for candle in &self.live_overlay_cache {
            observed_min = observed_min.min(candle.low);
            observed_max = observed_max.max(candle.high);
            max_volume = max_volume.max(candle.volume.max(0.0));
        }

        let (min_price, max_price) = if state.view_settings.auto_y_scale {
            if observed_min == f32::MAX || observed_max == f32::MIN {
                (state.fixed_y_min, state.fixed_y_max)
            } else {
                let range = (observed_max - observed_min).max(1e-6);
                let padding = range * 0.05;
                (observed_min - padding, observed_max + padding)
            }
        } else {
            (
                state.fixed_y_min + state.pan_offset_y,
                state.fixed_y_max + state.pan_offset_y,
            )
        };

        self.bars.update_instances_with_range(
            historical_slice,
            lod_level,
            queue,
            min_price,
            max_price,
            num_bars_in_viewport,
            -LEFT_PADDING_BARS,
            max_volume,
        );

        if self.live_overlay_cache.is_empty() {
            self.bars.clear_live_instances();
        } else {
            let first_live_position = historical_slice.len() as f32 - LEFT_PADDING_BARS;
            self.bars.update_live_instances_with_range(
                &self.live_overlay_cache,
                lod_level,
                queue,
                min_price,
                max_price,
                num_bars_in_viewport,
                first_live_position,
                max_volume,
            );
        }

        if !historical_slice.is_empty() {
            self.grid.update_grid_lines(
                queue,
                historical_slice,
                lod_level,
                min_price,
                max_price,
                num_bars_in_viewport,
                &state.dividends,
                visible_start_idx,
                state.ticker.as_deref(),
                state.title(),
            );
        } else {
            self.grid.update_grid_lines(
                queue,
                &self.live_overlay_cache,
                lod_level,
                min_price,
                max_price,
                num_bars_in_viewport,
                &state.dividends,
                0,
                state.ticker.as_deref(),
                state.title(),
            );
        }

        let marker_span = (max_price - min_price).max(1e-6);
        let marker_data = active_live_data
            .and_then(|data| data.vwap_marker)
            .and_then(|marker| {
                if num_bars_in_viewport == 0 {
                    return None;
                }

                let price_norm = ((marker.price - min_price) / marker_span) * 2.0 - 1.0;

                let volume = marker.volume.max(1.0);
                let diameter_px = (6.0 + volume.ln() * 2.5).clamp(4.0, 48.0);
                let radius_px = diameter_px * 0.5;

                let color = match marker.trend {
                    PriceTrend::Up => [0.2, 0.8, 0.3, 0.95],
                    PriceTrend::Down => [0.9, 0.2, 0.2, 0.95],
                    PriceTrend::Flat => [0.6, 0.6, 0.6, 0.9],
                };

                let live_count = self.live_overlay_cache.len();
                if live_count == 0 {
                    return None;
                }

                let last_live_index = live_count.saturating_sub(1) as f32;
                let bar_position = historical_slice.len() as f32 - LEFT_PADDING_BARS + last_live_index;
                let viewport_bars = num_bars_in_viewport.max(1) as f32;
                let tick_idx_rel = (bar_position / viewport_bars) * 2.0 - 1.0;

                Some(MarkerInstanceData {
                    tick_idx_rel,
                    price_norm: price_norm.clamp(-1.5, 1.5),
                    radius_px,
                    color,
                })
            });

        self.refresh_position_overlays(
            queue,
            state,
            lod_level,
            num_bars_in_viewport,
            historical_slice,
        );

        self.refresh_price_level_quads(
            queue,
            state,
            lod_level,
            num_bars_in_viewport,
            historical_slice,
            min_price,
            max_price,
        );

        self.refresh_line_overlays(
            queue,
            state,
            lod_level,
            num_bars_in_viewport,
            historical_slice,
            min_price,
            max_price,
        );

        self.bars.update_marker_instance(marker_data, queue);

        let elapsed = update_start.elapsed();
        if elapsed.as_millis() > 16 {
            eprintln!(
                "⚠ viewport update took {}ms (candles={}, live={}, bars_in_vp={})",
                elapsed.as_millis(),
                historical_slice.len(),
                self.live_overlay_cache.len(),
                num_bars_in_viewport,
            );
        }

        Some((min_price, max_price))
    }

    /// Convert a timestamp range to normalized X coordinates [-1, 1] in viewport space.
    ///
    /// Returns None if the range is invalid or falls outside the viewport.
    fn timestamp_range_to_x_coords(
        start_ts: i64,
        end_ts: i64,
        base_ts: f64,
        interval_ns: f64,
        num_bars_in_viewport: u32,
    ) -> Option<(f32, f32)> {
        // Calculate bar deltas from base timestamp
        let start_delta = ((start_ts as f64) - base_ts) / interval_ns;
        let end_delta = ((end_ts as f64) - base_ts) / interval_ns;

        // Ensure start is before end
        let (start_delta, end_delta) = if end_delta >= start_delta {
            (start_delta as f32, end_delta as f32)
        } else {
            (end_delta as f32, start_delta as f32)
        };

        // Apply left padding offset to align with bar rendering
        let start_pos = start_delta - LEFT_PADDING_BARS - 0.5;
        let end_pos = end_delta - LEFT_PADDING_BARS + 0.5;

        // Normalize to viewport coordinates [-1, 1]
        let viewport_bars = num_bars_in_viewport.max(1) as f32;
        let start_tick = (start_pos / viewport_bars) * 2.0 - 1.0;
        let end_tick = (end_pos / viewport_bars) * 2.0 - 1.0;

        // Check if range is valid
        if end_tick <= start_tick {
            return None;
        }

        // Clamp to visible range
        let clipped_start = start_tick.clamp(-1.0, 1.0);
        let clipped_end = end_tick.clamp(-1.0, 1.0);

        // Check if still valid after clipping
        if clipped_end <= clipped_start {
            return None;
        }

        Some((clipped_start, clipped_end))
    }

    fn refresh_position_overlays(
        &mut self,
        queue: &wgpu::Queue,
        state: &ViewportState,
        lod_level: crate::zoom::LodLevel,
        num_bars_in_viewport: u32,
        historical_slice: &[PlotCandle],
    ) {
        if state.position_overlays().is_empty() || num_bars_in_viewport == 0 {
            self.position_overlays.clear_instances();
            return;
        }

        let base_ts = if let Some(candle) = historical_slice.first() {
            candle.ts
        } else if let Some(candle) = self.live_overlay_cache.first() {
            candle.ts
        } else {
            self.position_overlays.clear_instances();
            return;
        };

        let interval_ns = (lod_level.seconds() as f64) * 1_000_000_000.0;
        if interval_ns <= 0.0 {
            self.position_overlays.clear_instances();
            return;
        }

        let base_ts = base_ts as f64;
        let mut instances = Vec::new();

        for overlay in state.position_overlays() {
            if !overlay.is_valid() {
                continue;
            }

            if let Some((x_start, x_end)) = Self::timestamp_range_to_x_coords(
                overlay.start_ts,
                overlay.end_ts,
                base_ts,
                interval_ns,
                num_bars_in_viewport,
            ) {
                instances.push(PositionOverlayInstanceData {
                    x_start,
                    x_end,
                    color: overlay.color,
                });
            }
        }

        if instances.is_empty() {
            self.position_overlays.clear_instances();
        } else {
            self.position_overlays.set_instances(&instances, queue);
        }
    }

    fn refresh_price_level_quads(
        &mut self,
        queue: &wgpu::Queue,
        state: &ViewportState,
        lod_level: crate::zoom::LodLevel,
        num_bars_in_viewport: u32,
        historical_slice: &[PlotCandle],
        min_price: f32,
        max_price: f32,
    ) {
        if state.price_level_quads().is_empty() || num_bars_in_viewport == 0 {
            self.price_level_quads.clear_instances();
            return;
        }

        let base_ts = if let Some(candle) = historical_slice.first() {
            candle.ts
        } else if let Some(candle) = self.live_overlay_cache.first() {
            candle.ts
        } else {
            self.price_level_quads.clear_instances();
            return;
        };

        let interval_ns = (lod_level.seconds() as f64) * 1_000_000_000.0;
        if interval_ns <= 0.0 {
            self.price_level_quads.clear_instances();
            return;
        }

        let base_ts = base_ts as f64;
        let mut instances = Vec::new();

        for quad in state.price_level_quads() {
            if !quad.is_valid() {
                continue;
            }

            // Calculate X range (time) using helper function
            if let Some((x_start, x_end)) = Self::timestamp_range_to_x_coords(
                quad.start_ts,
                quad.end_ts,
                base_ts,
                interval_ns,
                num_bars_in_viewport,
            ) {
                instances.push(PriceLevelQuadInstanceData {
                    x_start,
                    x_end,
                    y_start: quad.y_start,
                    y_end: quad.y_end,
                    color: quad.color,
                });
            }
        }

        if instances.is_empty() {
            self.price_level_quads.clear_instances();
        } else {
            self.price_level_quads.set_instances(&instances, queue, min_price, max_price);
        }
    }

    fn refresh_line_overlays(
        &mut self,
        queue: &wgpu::Queue,
        state: &ViewportState,
        lod_level: crate::zoom::LodLevel,
        num_bars_in_viewport: u32,
        historical_slice: &[PlotCandle],
        min_price: f32,
        max_price: f32,
    ) {
        if state.line_overlays().is_empty() || num_bars_in_viewport == 0 {
            self.line_overlays.clear();
            return;
        }

        let base_ts = if let Some(candle) = historical_slice.first() {
            candle.ts
        } else if let Some(candle) = self.live_overlay_cache.first() {
            candle.ts
        } else {
            self.line_overlays.clear();
            return;
        };

        let interval_ns = (lod_level.seconds() as f64) * 1_000_000_000.0;
        if interval_ns <= 0.0 {
            self.line_overlays.clear();
            return;
        }

        let base_ts_f = base_ts as f64;
        let viewport_bars = num_bars_in_viewport.max(1) as f32;

        let mut price_span = max_price - min_price;
        if price_span.abs() < 1e-6 {
            price_span = 1e-6;
        }
        let normalize_price = |price: f32| -> f32 {
            ((price - min_price) / price_span) * 2.0 - 1.0
        };

        let mut vertices = Vec::new();

        for overlay in state.line_overlays() {
            if overlay.points.len() < 2 {
                continue;
            }

            // Convert points to bar-space coordinates
            let mut converted: Vec<[f32; 2]> = Vec::with_capacity(overlay.points.len());
            for &(ts, price) in &overlay.points {
                let delta = ((ts as f64) - base_ts_f) / interval_ns;
                let x_pos = delta as f32 - LEFT_PADDING_BARS;
                let x_norm = (x_pos / viewport_bars) * 2.0 - 1.0;
                let y_norm = normalize_price(price);
                converted.push([x_norm, y_norm]);
            }

            // Emit LineList pairs for adjacent points
            for pair in converted.windows(2) {
                // Skip segments entirely outside viewport
                if (pair[0][0] > 1.0 && pair[1][0] > 1.0)
                    || (pair[0][0] < -1.0 && pair[1][0] < -1.0)
                {
                    continue;
                }
                vertices.push(LineVertex {
                    position: pair[0],
                    color: overlay.color,
                });
                vertices.push(LineVertex {
                    position: pair[1],
                    color: overlay.color,
                });
            }
        }

        if vertices.is_empty() {
            self.line_overlays.clear();
        } else {
            self.line_overlays.set_vertices(&vertices, queue);
        }
    }

    pub fn resize(
        &mut self,
        state: &ViewportState,
        queue: &wgpu::Queue,
        window_w: f32,
        window_h: f32,
    ) {
        self.bars.view_uniform.viewport = [state.x, state.y, state.width, state.height];
        self.bars.view_uniform.window = [window_w, window_h, state.zoom.bar_width_px as f32, 0.0];
        self.bars
            .set_volume_enabled(state.view_settings.show_volume_bars);
        self.bars.update_view_uniform(queue);
        self.position_overlays
            .update_view_uniform(&self.bars.view_uniform, queue);
        self.price_level_quads
            .update_view_uniform(&self.bars.view_uniform, queue);
        self.line_overlays
            .update_view_uniform(&self.bars.view_uniform, queue);

        self.grid.update_uniforms(
            queue,
            state.x,
            state.y,
            state.width,
            state.height,
            window_w,
            window_h,
        );
    }

    pub fn draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        self.position_overlays.draw(render_pass);
        self.price_level_quads.draw(render_pass);
        self.line_overlays.draw(render_pass);
        self.grid.draw(render_pass);
        self.bars.draw(render_pass);
    }

    pub fn draw_labels<'a>(
        &'a mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_pass: &mut wgpu::RenderPass<'a>,
        glyphon_viewport: &glyphon::Viewport,
    ) {
        self.grid
            .draw_labels(device, queue, render_pass, glyphon_viewport);
    }
}
