use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use winit::window::Window;

use crate::{
    MarketData,
    live::LiveDataSource,
    loader::load_market_data_with_config,
    state::ViewportState,
    view::{tooltip::TooltipRenderer, viewport::ViewportRenderer, viewport_view::ViewportView},
};

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    viewport: ViewportRenderer,

    // Model: Pure state
    pub viewport_states: Vec<ViewportState>,

    // View: GPU renderers
    viewport_views: Vec<ViewportView>,
    last_live_tick: Instant,

    // Glyphon for text rendering — held for lifetime management
    #[allow(dead_code)]
    glyphon_cache: glyphon::Cache,
    glyphon_viewport: glyphon::Viewport,

    // Focus tracking
    focused_viewport_idx: usize,

    // Offscreen framebuffer for partial redraws
    framebuffer: wgpu::Texture,
    framebuffer_view: wgpu::TextureView,
    framebuffer_initialized: bool,

    // Dirty tracking
    dirty_viewports: Vec<bool>,
    full_redraw_pending: bool,

    // Vizza configuration
    vizza_config: crate::Config,

    // Color palette from theme
    palette: crate::config::ColorPalette,

    // Tooltip renderer
    tooltip: TooltipRenderer,

    // Current hover state for tooltip
    hover_viewport_idx: Option<usize>,
    hover_bar_index: Option<usize>,
    hover_mouse_pos: (f32, f32),
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        vizza_config: crate::Config,
        custom_live_source: Option<Box<dyn LiveDataSource>>,
        use_live_data: bool,
        preloaded_market_data: Option<MarketData>,
        per_viewport_market_data: Option<Vec<MarketData>>,
        today_so_far_enabled: bool,
    ) -> Result<Self> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("Failed to find adapter"))?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &config);

        // Get grid dimensions from config
        let grid_rows = vizza_config.grid_rows;
        let grid_cols = vizza_config.grid_cols;
        let total_viewports = grid_rows * grid_cols;

        // Get color palette from theme
        let palette = crate::config::ColorPalette::from_theme(vizza_config.theme);

        println!(
            "Renderer: Creating {}x{} grid ({} viewports)",
            grid_rows,
            grid_cols,
            grid_rows * grid_cols
        );
        println!(
            "Renderer: allow_missing_history = {}",
            vizza_config.allow_missing_history
        );

        // Check if we should auto-pan to end (for live data)
        let has_custom_live_source = custom_live_source.is_some();
        if has_custom_live_source || use_live_data {
            println!("Renderer: Auto-panning to end of data for live viewing");
        }

        let market_data_vec = if let Some(datasets) = per_viewport_market_data {
            if datasets.len() != total_viewports {
                return Err(anyhow::anyhow!(
                    "Expected {} in-memory datasets, received {}",
                    total_viewports,
                    datasets.len()
                ));
            }
            println!(
                "Renderer: Using {} preloaded viewport datasets",
                datasets.len()
            );
            datasets
        } else {
            let market_data = if let Some(market_data) = preloaded_market_data {
                println!("Renderer: Using shared preloaded in-memory market data");
                market_data
            } else {
                load_market_data_with_config(&vizza_config)?
            };
            vec![market_data.clone(); total_viewports]
        };

        // Grid layout with margin and gaps
        let viewport_margin = 50.0;
        let viewport_gap = 8.0;

        // Calculate viewport dimensions
        let total_width =
            size.width as f32 - 2.0 * viewport_margin - (grid_cols as f32 - 1.0) * viewport_gap;
        let total_height =
            size.height as f32 - 2.0 * viewport_margin - (grid_rows as f32 - 1.0) * viewport_gap;
        let viewport_w = total_width / grid_cols as f32;
        let viewport_h = total_height / grid_rows as f32;

        // For the first viewport (used for background/border rendering template)
        let first_viewport_x = viewport_margin;
        let first_viewport_y = viewport_margin;

        let viewport = ViewportRenderer::new(
            &device,
            config.format,
            first_viewport_x,
            first_viewport_y,
            viewport_w,
            viewport_h,
            size.width as f32,
            size.height as f32,
            palette.viewport_bg,
        );

        let base_view_settings = vizza_config.view_settings.clone();

        // Create viewport states in a grid (model - pure state, no GPU)
        let mut viewport_states = Vec::new();
        for row in 0..grid_rows {
            for col in 0..grid_cols {
                let idx = row * grid_cols + col;
                let market_data = &market_data_vec[idx];

                let viewport_x = viewport_margin + col as f32 * (viewport_w + viewport_gap);
                let viewport_y = viewport_margin + row as f32 * (viewport_h + viewport_gap);

                // Determine initial left timestamp for this viewport:
                // Per-viewport start time takes precedence over global
                let initial_left_ts = vizza_config
                    .initial_left_times
                    .get(idx)
                    .copied()
                    .flatten()
                    .or(vizza_config.initial_left_ts);

                let mut state = ViewportState::new(
                    viewport_x,
                    viewport_y,
                    viewport_w,
                    viewport_h,
                    market_data,
                    initial_left_ts,
                    vizza_config.default_lod_level,
                );

                state.view_settings = base_view_settings.clone();

                state.zoom.bar_width_px = vizza_config.bar_width_px;

                if let Some(overlays) = vizza_config.position_overlays.get(idx) {
                    state.set_position_overlays(overlays.clone());
                }

                if let Some(quads) = vizza_config.price_level_quads.get(idx) {
                    state.set_price_level_quads(quads.clone());
                }

                if let Some(lines) = vizza_config.line_overlays.get(idx) {
                    state.set_line_overlays(lines.clone());
                }

                if let Some(ticker) = vizza_config
                    .tickers
                    .get(idx)
                    .and_then(|t| t.as_ref().cloned())
                {
                    state.set_ticker(ticker);
                }

                if let Some(title) = vizza_config
                    .titles
                    .get(idx)
                    .and_then(|t| t.as_ref().cloned())
                {
                    state.set_title(title);
                }

                // Auto-pan to end for live data mode or when there's no historical data
                if has_custom_live_source || use_live_data {
                    state.pan_to_end_with_lod();

                    // Set minimum LOD to 5 seconds for live data (finest live granularity)
                    state.zoom.set_min_lod_from_interval(5);
                }

                viewport_states.push(state);
            }
        }

        // Create viewport views (view - GPU renderers)
        let mut viewport_views = Vec::new();
        let mut custom_source_option = custom_live_source;
        for (idx, state) in viewport_states.iter().enumerate() {
            let level_store = Arc::clone(&market_data_vec[idx].level_store);
            // Use custom data source for the first viewport if provided
            let view = if idx == 0 && custom_source_option.is_some() {
                let source = custom_source_option.take();
                ViewportView::new_with_custom_source(
                    &device,
                    &queue,
                    config.format,
                    state,
                    size.width as f32,
                    size.height as f32,
                    level_store,
                    source,
                    "UNKNOWN", // TODO: Get ticker from config
                    today_so_far_enabled,
                    &palette,
                )
            } else {
                ViewportView::new(
                    &device,
                    &queue,
                    config.format,
                    state,
                    size.width as f32,
                    size.height as f32,
                    level_store,
                    use_live_data,
                    &palette,
                )
            };
            viewport_views.push(view);
        }

        // Initialize views with state data
        for i in 0..viewport_states.len() {
            let y_range = viewport_views[i].update(&viewport_states[i], &queue);
            if let Some((y_min, y_max)) = y_range {
                viewport_states[i].update_y_range(y_min, y_max);
            }
        }

        // If today-so-far backfill was enabled, re-pan to end to show the injected bars
        if today_so_far_enabled && has_custom_live_source {
            println!("Re-panning viewport to show backfill bars...");
            for state in &mut viewport_states {
                state.pan_to_end_with_lod();
            }
        }

        // Initialize glyphon
        let glyphon_cache = glyphon::Cache::new(&device);
        let mut glyphon_viewport = glyphon::Viewport::new(&device, &glyphon_cache);
        glyphon_viewport.update(
            &queue,
            glyphon::Resolution {
                width: size.width,
                height: size.height,
            },
        );

        let dirty_viewports = vec![true; total_viewports];
        let (framebuffer, framebuffer_view) = Self::create_framebuffer(
            &device,
            config.format,
            config.width,
            config.height,
        );

        // Initialize tooltip renderer
        let tooltip = TooltipRenderer::new(
            &device,
            &queue,
            config.format,
            size.width as f32,
            size.height as f32,
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            viewport,
            viewport_states,
            viewport_views,
            last_live_tick: Instant::now(),
            glyphon_cache,
            glyphon_viewport,
            focused_viewport_idx: 0,
            framebuffer,
            framebuffer_view,
            framebuffer_initialized: false,
            dirty_viewports,
            full_redraw_pending: true,
            vizza_config,
            palette,
            tooltip,
            hover_viewport_idx: None,
            hover_bar_index: None,
            hover_mouse_pos: (0.0, 0.0),
        })
    }

    pub async fn new_with_sources(
        window: Arc<Window>,
        vizza_config: crate::Config,
        custom_live_sources: Vec<(String, Box<dyn LiveDataSource>)>,
        use_live_data: bool,
        preloaded_market_data: Option<MarketData>,
        per_viewport_market_data: Option<Vec<MarketData>>,
        today_so_far_enabled: bool,
    ) -> Result<Self> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("Failed to find adapter"))?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &config);

        // Get grid dimensions from config
        let grid_rows = vizza_config.grid_rows;
        let grid_cols = vizza_config.grid_cols;
        let total_viewports = grid_rows * grid_cols;

        // Get color palette from theme
        let palette = crate::config::ColorPalette::from_theme(vizza_config.theme);

        println!(
            "Renderer: Creating {}x{} grid ({} viewports)",
            grid_rows,
            grid_cols,
            grid_rows * grid_cols
        );
        println!(
            "Renderer: allow_missing_history = {}",
            vizza_config.allow_missing_history
        );
        println!(
            "Renderer: {} custom live sources provided",
            custom_live_sources.len()
        );

        // Check if we should auto-pan to end (for live data)
        let has_custom_live_sources = !custom_live_sources.is_empty();
        if has_custom_live_sources || use_live_data {
            println!("Renderer: Auto-panning to end of data for live viewing");
        }

        let market_data_vec = if let Some(datasets) = per_viewport_market_data {
            if datasets.len() != total_viewports {
                return Err(anyhow::anyhow!(
                    "Expected {} in-memory datasets, received {}",
                    total_viewports,
                    datasets.len()
                ));
            }
            println!(
                "Renderer: Using {} preloaded viewport datasets",
                datasets.len()
            );
            datasets
        } else {
            let market_data = if let Some(market_data) = preloaded_market_data {
                println!("Renderer: Using shared preloaded in-memory market data");
                market_data
            } else {
                load_market_data_with_config(&vizza_config)?
            };
            vec![market_data.clone(); total_viewports]
        };

        // Grid layout with margin and gaps
        let viewport_margin = 50.0;
        let viewport_gap = 8.0;

        // Calculate viewport dimensions
        let total_width =
            size.width as f32 - 2.0 * viewport_margin - (grid_cols as f32 - 1.0) * viewport_gap;
        let total_height =
            size.height as f32 - 2.0 * viewport_margin - (grid_rows as f32 - 1.0) * viewport_gap;
        let viewport_w = total_width / grid_cols as f32;
        let viewport_h = total_height / grid_rows as f32;

        // For the first viewport (used for background/border rendering template)
        let first_viewport_x = viewport_margin;
        let first_viewport_y = viewport_margin;

        let viewport = ViewportRenderer::new(
            &device,
            config.format,
            first_viewport_x,
            first_viewport_y,
            viewport_w,
            viewport_h,
            size.width as f32,
            size.height as f32,
            palette.viewport_bg,
        );

        // Create viewport states in a grid (model - pure state, no GPU)
        let mut viewport_states = Vec::new();
        let base_view_settings = vizza_config.view_settings.clone();
        let sources_iter = custom_live_sources.into_iter();

        for row in 0..grid_rows {
            for col in 0..grid_cols {
                let idx = row * grid_cols + col;
                let market_data = &market_data_vec[idx];
                let viewport_x = viewport_margin + col as f32 * (viewport_w + viewport_gap);
                let viewport_y = viewport_margin + row as f32 * (viewport_h + viewport_gap);

                // Determine initial left timestamp for this viewport:
                // Per-viewport start time takes precedence over global
                let initial_left_ts = vizza_config
                    .initial_left_times
                    .get(idx)
                    .copied()
                    .flatten()
                    .or(vizza_config.initial_left_ts);

                let mut state = ViewportState::new(
                    viewport_x,
                    viewport_y,
                    viewport_w,
                    viewport_h,
                    market_data,
                    initial_left_ts,
                    vizza_config.default_lod_level,
                );

                state.view_settings = base_view_settings.clone();

                state.zoom.bar_width_px = vizza_config.bar_width_px;

                if let Some(overlays) = vizza_config.position_overlays.get(idx) {
                    state.set_position_overlays(overlays.clone());
                }

                if let Some(quads) = vizza_config.price_level_quads.get(idx) {
                    state.set_price_level_quads(quads.clone());
                }

                if let Some(lines) = vizza_config.line_overlays.get(idx) {
                    state.set_line_overlays(lines.clone());
                }

                if let Some(ticker) = vizza_config
                    .tickers
                    .get(idx)
                    .and_then(|t| t.as_ref().cloned())
                {
                    state.set_ticker(ticker);
                }

                if let Some(title) = vizza_config
                    .titles
                    .get(idx)
                    .and_then(|t| t.as_ref().cloned())
                {
                    state.set_title(title);
                }

                // Auto-pan to end for live data mode or when there's no historical data
                if has_custom_live_sources || use_live_data {
                    state.pan_to_end_with_lod();

                    // Set minimum LOD to 5 seconds for live data (finest live granularity)
                    state.zoom.set_min_lod_from_interval(5);
                }

                viewport_states.push(state);
            }
        }

        // Create viewport views (view - GPU renderers)
        // Distribute custom sources to viewports, one per viewport
        // Also set the ticker on the corresponding state
        let mut viewport_views = Vec::new();
        let mut sources_with_ticker: Vec<(String, Box<dyn LiveDataSource>)> =
            sources_iter.collect();

        // DEBUG: Print source distribution info
        eprintln!(
            "DEBUG Renderer: Total viewports = {}",
            viewport_states.len()
        );
        eprintln!(
            "DEBUG Renderer: sources_with_ticker.len() = {}",
            sources_with_ticker.len()
        );
        eprintln!(
            "DEBUG Renderer: today_so_far_enabled = {}",
            today_so_far_enabled
        );

        for (idx, state) in viewport_states.iter_mut().enumerate() {
            eprintln!(
                "DEBUG Renderer: Processing viewport {} (sources remaining: {})",
                idx,
                sources_with_ticker.len()
            );
            let level_store = Arc::clone(&market_data_vec[idx].level_store);
            let view = if !sources_with_ticker.is_empty() {
                let (ticker, source) = sources_with_ticker.swap_remove(0);
                // Set ticker on state
                state.set_ticker(ticker.clone());
                // Use custom data source with ticker for this viewport
                println!(
                    "Renderer: Viewport {} using custom source for ticker {}",
                    idx, ticker
                );
                eprintln!(
                    "DEBUG Renderer: Viewport {} assigned ticker: {}",
                    idx, ticker
                );
                ViewportView::new_with_custom_source(
                    &device,
                    &queue,
                    config.format,
                    state,
                    size.width as f32,
                    size.height as f32,
                    level_store,
                    Some(source),
                    &ticker,
                    today_so_far_enabled,
                    &palette,
                )
            } else {
                // No more custom sources, use default source
                eprintln!(
                    "DEBUG Renderer: Viewport {} using default source (no ticker)",
                    idx
                );
                ViewportView::new(
                    &device,
                    &queue,
                    config.format,
                    state,
                    size.width as f32,
                    size.height as f32,
                    level_store,
                    use_live_data,
                    &palette,
                )
            };
            viewport_views.push(view);
        }

        // Initialize views with state data
        for i in 0..viewport_states.len() {
            let y_range = viewport_views[i].update(&viewport_states[i], &queue);
            if let Some((y_min, y_max)) = y_range {
                viewport_states[i].update_y_range(y_min, y_max);
            }
        }

        // If today-so-far backfill was enabled, re-pan to end to show the injected bars
        if today_so_far_enabled && has_custom_live_sources {
            println!("Re-panning viewport to show backfill bars...");
            for state in &mut viewport_states {
                state.pan_to_end_with_lod();
            }
        }

        // Initialize glyphon
        let glyphon_cache = glyphon::Cache::new(&device);
        let mut glyphon_viewport = glyphon::Viewport::new(&device, &glyphon_cache);
        glyphon_viewport.update(
            &queue,
            glyphon::Resolution {
                width: size.width,
                height: size.height,
            },
        );

        let dirty_viewports = vec![true; total_viewports];
        let (framebuffer, framebuffer_view) = Self::create_framebuffer(
            &device,
            config.format,
            config.width,
            config.height,
        );

        // Initialize tooltip renderer
        let tooltip = TooltipRenderer::new(
            &device,
            &queue,
            config.format,
            size.width as f32,
            size.height as f32,
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            viewport,
            viewport_states,
            viewport_views,
            last_live_tick: Instant::now(),
            glyphon_cache,
            glyphon_viewport,
            focused_viewport_idx: 0,
            framebuffer,
            framebuffer_view,
            framebuffer_initialized: false,
            dirty_viewports,
            full_redraw_pending: true,
            vizza_config,
            palette,
            tooltip,
            hover_viewport_idx: None,
            hover_bar_index: None,
            hover_mouse_pos: (0.0, 0.0),
        })
    }

    fn create_framebuffer(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Vizza Framebuffer"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn recreate_framebuffer(&mut self) {
        let (framebuffer, framebuffer_view) = Self::create_framebuffer(
            &self.device,
            self.config.format,
            self.config.width,
            self.config.height,
        );
        self.framebuffer = framebuffer;
        self.framebuffer_view = framebuffer_view;
        self.framebuffer_initialized = false;
        self.mark_all_viewports_dirty();
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);

            // Get grid dimensions from config
            let grid_rows = self.vizza_config.grid_rows;
            let grid_cols = self.vizza_config.grid_cols;

            // Grid layout with margin and gaps
            let viewport_margin = 50.0;
            let viewport_gap = 8.0;

            // Calculate viewport dimensions
            let total_width = new_size.width as f32
                - 2.0 * viewport_margin
                - (grid_cols as f32 - 1.0) * viewport_gap;
            let total_height = new_size.height as f32
                - 2.0 * viewport_margin
                - (grid_rows as f32 - 1.0) * viewport_gap;
            let viewport_w = total_width / grid_cols as f32;
            let viewport_h = total_height / grid_rows as f32;

            let window_w = new_size.width as f32;
            let window_h = new_size.height as f32;

            // Update viewport states (model) in grid layout
            let mut idx = 0;
            for row in 0..grid_rows {
                for col in 0..grid_cols {
                    if idx >= self.viewport_states.len() {
                        break;
                    }
                    let viewport_x = viewport_margin + col as f32 * (viewport_w + viewport_gap);
                    let viewport_y = viewport_margin + row as f32 * (viewport_h + viewport_gap);

                    self.viewport_states[idx]
                        .update_size(viewport_x, viewport_y, viewport_w, viewport_h);
                    idx += 1;
                }
            }

            // Update viewport views (view)
            for i in 0..self.viewport_states.len() {
                self.viewport_views[i].resize(
                    &self.viewport_states[i],
                    &self.queue,
                    window_w,
                    window_h,
                );
                let y_range = self.viewport_views[i].update(&self.viewport_states[i], &self.queue);
                if let Some((y_min, y_max)) = y_range {
                    self.viewport_states[i].update_y_range(y_min, y_max);
                }
            }

            // Update viewport renderer (use first viewport for template)
            let first_viewport_x = viewport_margin;
            let first_viewport_y = viewport_margin;

            self.viewport.bg_uniform.viewport_x = first_viewport_x;
            self.viewport.bg_uniform.viewport_y = first_viewport_y;
            self.viewport.bg_uniform.viewport_w = viewport_w;
            self.viewport.bg_uniform.viewport_h = viewport_h;
            self.viewport.bg_uniform.window_w = window_w;
            self.viewport.bg_uniform.window_h = window_h;
            self.viewport.update_bg_uniform(&self.queue);

            self.viewport.border_uniform.viewport_x = first_viewport_x;
            self.viewport.border_uniform.viewport_y = first_viewport_y;
            self.viewport.border_uniform.viewport_w = viewport_w;
            self.viewport.border_uniform.viewport_h = viewport_h;
            self.viewport.border_uniform.window_w = window_w;
            self.viewport.border_uniform.window_h = window_h;
            self.viewport.update_border_uniform(&self.queue);

            // Update glyphon viewport
            self.glyphon_viewport.update(
                &self.queue,
                glyphon::Resolution {
                    width: new_size.width,
                    height: new_size.height,
                },
            );

            // Update tooltip window size
            self.tooltip.update_window_size(window_w, window_h);

            self.recreate_framebuffer();
        }
    }

    pub fn has_pending_redraw(&self) -> bool {
        self.full_redraw_pending
            || !self.framebuffer_initialized
            || self.dirty_viewports.iter().any(|dirty| *dirty)
    }

    pub fn mark_viewport_dirty(&mut self, idx: usize) -> bool {
        if idx >= self.dirty_viewports.len() {
            return false;
        }

        if !self.dirty_viewports[idx] {
            self.dirty_viewports[idx] = true;
            true
        } else {
            false
        }
    }

    pub fn mark_all_viewports_dirty(&mut self) {
        self.full_redraw_pending = true;
        for dirty in &mut self.dirty_viewports {
            *dirty = true;
        }
    }

    /// Find which viewport contains the given point (x, y)
    pub fn find_viewport_index_at(&self, x: f64, y: f64) -> Option<usize> {
        self.viewport_states.iter().position(|vp| {
            let x = x as f32;
            let y = y as f32;
            x >= vp.x && x <= vp.x + vp.width && y >= vp.y && y <= vp.y + vp.height
        })
    }

    /// Get the bar index at a given X position within a viewport
    /// Returns the index into the visible candle range
    pub fn get_bar_index_at(&self, viewport_idx: usize, mouse_x: f64) -> Option<usize> {
        let state = self.viewport_states.get(viewport_idx)?;

        let bar_spacing = state.zoom.bar_width_px as f64 + 1.0;
        if bar_spacing <= 0.0 {
            return None;
        }

        let x_in_viewport = mouse_x - state.x as f64;
        if x_in_viewport < 0.0 || x_in_viewport >= state.width as f64 {
            return None;
        }

        // Calculate which bar position the mouse is over
        // bar_position = 0 corresponds to the first fully visible bar (at x = 0)
        // which is visible_candle[1] (since first_bar_position = -LEFT_PADDING_BARS = -1)
        let bar_position = (x_in_viewport / bar_spacing).floor() as i64;

        // Convert bar_position to index in the visible slice
        // visible_candle[0] is at bar_position -1, so bar_index = bar_position + 1
        let bar_index = (bar_position + 1) as usize;

        // Verify the bar index is within the visible range
        if let Some((_candles, start_idx, end_idx)) = state.get_visible_candle_range() {
            let visible_count = end_idx.saturating_sub(start_idx);
            if bar_index < visible_count {
                Some(bar_index)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Update hover state and tooltip
    pub fn update_hover(
        &mut self,
        viewport_idx: Option<usize>,
        bar_index: Option<usize>,
        mouse_x: f32,
        mouse_y: f32,
        show_tooltip: bool,
    ) {
        self.hover_viewport_idx = viewport_idx;
        self.hover_bar_index = bar_index;
        self.hover_mouse_pos = (mouse_x, mouse_y);

        // Only show tooltip if middle button is pressed
        if !show_tooltip {
            self.tooltip.update(&self.queue, None, mouse_x, mouse_y, None);
            return;
        }

        // Get candle data and viewport bounds if hovering over a bar
        let (tooltip_data, viewport_bounds) = if let (Some(vp_idx), Some(bar_idx)) = (viewport_idx, bar_index) {
            if let Some(state) = self.viewport_states.get(vp_idx) {
                let bounds = crate::view::tooltip::ViewportBounds {
                    x: state.x,
                    y: state.y,
                    width: state.width,
                    height: state.height,
                };
                if let Some((candles, start_idx, end_idx)) = state.get_visible_candle_range() {
                    let visible_count = end_idx.saturating_sub(start_idx);
                    if bar_idx < visible_count {
                        let candle_idx = start_idx + bar_idx;
                        if candle_idx < candles.len() {
                            (Some(crate::view::tooltip::TooltipData::from(&candles[candle_idx])), Some(bounds))
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        self.tooltip.update(&self.queue, tooltip_data.as_ref(), mouse_x, mouse_y, viewport_bounds);
    }

    /// Set the focused viewport and mark affected viewports dirty
    pub fn set_focused_viewport(&mut self, idx: usize) -> bool {
        if idx >= self.viewport_states.len() || self.viewport_states.is_empty() {
            return false;
        }

        if idx == self.focused_viewport_idx {
            return false;
        }

        let previous = self.focused_viewport_idx;
        self.focused_viewport_idx = idx;
        self.mark_viewport_dirty(previous);
        self.mark_viewport_dirty(idx);
        true
    }

    /// Get the focused viewport index
    pub fn focused_viewport_idx(&self) -> usize {
        self.focused_viewport_idx
    }

    pub fn handle_pan_viewport(&mut self, idx: usize, dx: f64, dy: f64, rescale_y: bool) {
        // Make sure viewport index is valid
        if idx >= self.viewport_states.len() {
            return;
        }

        // Update model (state)
        self.viewport_states[idx].handle_pan(dx, dy, rescale_y);
        self.mark_viewport_dirty(idx);
    }

    pub fn handle_scroll(
        &mut self,
        scroll_delta: f64,
        mouse_x: f64,
        mouse_y: f64,
        ctrl_pressed: bool,
    ) -> bool {
        // Find the viewport that contains the mouse cursor
        if let Some(idx) = self.find_viewport_index_at(mouse_x, mouse_y) {
            if ctrl_pressed {
                let state = &mut self.viewport_states[idx];

                let mut target_info: Option<(i64, usize)> = None;
                let bar_spacing = state.zoom.bar_width_px as f64 + 1.0;
                let x_in_viewport = mouse_x - state.x as f64;

                if bar_spacing > 0.0 && x_in_viewport >= 0.0 && x_in_viewport < state.width as f64 {
                    let bar_index = (x_in_viewport / bar_spacing).floor();
                    if bar_index >= 0.0 {
                        let bar_index = bar_index as usize;
                        if let Some((candles, start_idx, end_idx)) =
                            state.get_visible_candle_range()
                        {
                            let visible_count = end_idx.saturating_sub(start_idx);
                            if visible_count > 0 {
                                let clamped_index = bar_index.min(visible_count - 1);
                                let target_idx = start_idx + clamped_index;
                                if target_idx < candles.len() {
                                    target_info = Some((candles[target_idx].ts, clamped_index));
                                }
                            }
                        }
                    }
                }

                let available_lods = state.lod_levels_with_data();
                state.zoom.handle_lod_change(scroll_delta, &available_lods);
                println!(
                    "Viewport {}: LOD = {}",
                    idx,
                    state.zoom.current_lod_level.label()
                );

                if let Some((ts, relative_idx)) = target_info {
                    state.align_timestamp_to_relative_index(ts, relative_idx);
                }
            } else {
                // Regular scroll: adjust date span keeping bar under mouse constant
                let state = &mut self.viewport_states[idx];

                // Calculate which bar the mouse is over (for future enhancement)
                let bar_width = state.zoom.bar_width_px as f64;
                let gap = 1.0; // 1 pixel gap between bars
                let bar_spacing = bar_width + gap;

                // Calculate time per bar based on current LOD
                let seconds_per_bar = state.zoom.current_lod_level.seconds() as i64;

                // Scroll sensitivity: each scroll tick moves by 10% of visible range
                let num_bars_visible = state.width as f64 / bar_spacing;
                let scroll_time = (num_bars_visible * 0.1 * seconds_per_bar as f64) as i64;

                // Update viewport position (positive scroll_delta = scroll forward in time = increase right ts)
                state.viewport_right_ts -= (scroll_delta * scroll_time as f64) as i64;

                // Clamp to valid range based on actual data bounds
                let (min_ts, max_ts) = state.calculate_pan_limits();
                state.viewport_right_ts = state.viewport_right_ts.clamp(min_ts, max_ts);

                println!(
                    "Viewport {}: right_ts = {} (limits: {} to {})",
                    idx, state.viewport_right_ts, min_ts, max_ts
                );
            }

            // Update the viewport's rendering based on new state
            let y_range = self.viewport_views[idx].update(&self.viewport_states[idx], &self.queue);
            if let Some((y_min, y_max)) = y_range {
                self.viewport_states[idx].update_y_range(y_min, y_max);
            }

            self.mark_viewport_dirty(idx);
            return true;
        }
        false
    }

    pub fn update_instances(&mut self) {
        for i in 0..self.viewport_states.len() {
            self.update_viewport_instances(i);
        }
    }

    pub fn update_viewport_instances(&mut self, idx: usize) {
        if idx >= self.viewport_states.len() {
            return;
        }

        let y_range = self.viewport_views[idx].update(&self.viewport_states[idx], &self.queue);
        if let Some((y_min, y_max)) = y_range {
            self.viewport_states[idx].update_y_range(y_min, y_max);
        }
        self.mark_viewport_dirty(idx);
    }

    /// Toggle grid visibility for a specific viewport
    pub fn toggle_grid_visibility(&mut self, idx: usize) {
        if idx >= self.viewport_views.len() {
            return;
        }
        self.viewport_views[idx].grid.toggle_visibility();
        let visible = self.viewport_views[idx].grid.is_visible();
        println!(
            "Viewport {}: Grid {}",
            idx,
            if visible { "ON" } else { "OFF" }
        );
        self.mark_viewport_dirty(idx);
    }

    pub fn tick_live(&mut self) -> bool {
        let now = Instant::now();
        let dt = now - self.last_live_tick;
        self.last_live_tick = now;

        let mut updated = false;
        let mut dirty_idxs = Vec::new();
        for (idx, view) in self.viewport_views.iter_mut().enumerate() {
            if let Some(manager) = view.live_manager.as_mut() {
                if manager.update(dt) {
                    updated = true;
                    dirty_idxs.push(idx);
                }
            }
        }

        for idx in dirty_idxs {
            self.mark_viewport_dirty(idx);
        }

        updated
    }

    pub fn render(&mut self) -> Result<()> {
        let render_start = std::time::Instant::now();

        if self.viewport_states.is_empty() {
            return Ok(());
        }

        let force_full = self.full_redraw_pending || !self.framebuffer_initialized;
        let mut dirty_indices = Vec::new();

        if force_full {
            dirty_indices.extend(0..self.viewport_states.len());
            for dirty in &mut self.dirty_viewports {
                *dirty = false;
            }
        } else {
            for (idx, dirty) in self.dirty_viewports.iter_mut().enumerate() {
                if *dirty {
                    dirty_indices.push(idx);
                    *dirty = false;
                }
            }
        }

        if dirty_indices.is_empty() {
            return Ok(());
        }

        let mut first_pass = true;
        for &idx in &dirty_indices {
            let clear_frame = first_pass && (force_full || !self.framebuffer_initialized);
            self.render_viewport_into_framebuffer(idx, clear_frame)?;
            first_pass = false;
        }

        self.render_labels_into_framebuffer(&dirty_indices)?;

        // Render tooltip if visible
        if self.tooltip.is_visible() {
            self.render_tooltip_into_framebuffer()?;
        }

        self.full_redraw_pending = false;
        self.framebuffer_initialized = true;

        let output = self.surface.get_current_texture()?;
        self.copy_framebuffer_to_surface(&output);
        output.present();

        let elapsed = render_start.elapsed();
        if elapsed.as_millis() > 32 {
            eprintln!(
                "⚠ render took {}ms ({} dirty viewports)",
                elapsed.as_millis(),
                dirty_indices.len(),
            );
        }

        Ok(())
    }

    fn render_viewport_into_framebuffer(&mut self, idx: usize, clear_frame: bool) -> Result<()> {
        let y_range = {
            let state_ref = &self.viewport_states[idx];
            let viewport_view = &mut self.viewport_views[idx];
            viewport_view.update(state_ref, &self.queue)
        };

        if let Some((y_min, y_max)) = y_range {
            self.viewport_states[idx].update_y_range(y_min, y_max);
        }

        let state = &self.viewport_states[idx];
        let viewport_view = &self.viewport_views[idx];

        // Update viewport background/border uniforms
        self.viewport.bg_uniform.viewport_x = state.x;
        self.viewport.bg_uniform.viewport_y = state.y;
        self.viewport.bg_uniform.viewport_w = state.width;
        self.viewport.bg_uniform.viewport_h = state.height;
        self.viewport.update_bg_uniform(&self.queue);

        self.viewport.border_uniform.viewport_x = state.x;
        self.viewport.border_uniform.viewport_y = state.y;
        self.viewport.border_uniform.viewport_w = state.width;
        self.viewport.border_uniform.viewport_h = state.height;
        self.viewport.border_uniform.border_color = if idx == self.focused_viewport_idx {
            [1.0, 1.0, 1.0]
        } else {
            [0.3, 0.3, 0.3]
        };
        self.viewport.update_border_uniform(&self.queue);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Viewport Render Encoder"),
            });

        let load_op = if clear_frame {
            wgpu::LoadOp::Clear(wgpu::Color {
                r: self.palette.background[0] as f64,
                g: self.palette.background[1] as f64,
                b: self.palette.background[2] as f64,
                a: 1.0,
            })
        } else {
            wgpu::LoadOp::Load
        };

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Viewport Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.framebuffer_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: load_op,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
        self.viewport.draw_background(&mut render_pass);

        render_pass.set_scissor_rect(
            state.x as u32,
            state.y as u32,
            state.width as u32,
            state.height as u32,
        );
        viewport_view.draw(&mut render_pass);

        render_pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
        self.viewport.draw_border(&mut render_pass);

        drop(render_pass);

        self.queue.submit(std::iter::once(encoder.finish()));

        Ok(())
    }

    fn render_labels_into_framebuffer(&mut self, dirty_indices: &[usize]) -> Result<()> {
        if dirty_indices.is_empty() {
            return Ok(());
        }

        let mut label_encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Framebuffer Label Encoder"),
                });

        for &idx in dirty_indices {
            let mut label_pass = label_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Framebuffer Label Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.framebuffer_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            {
                let viewport_view = &mut self.viewport_views[idx];
                viewport_view.draw_labels(
                    &self.device,
                    &self.queue,
                    &mut label_pass,
                    &self.glyphon_viewport,
                );
            }

            drop(label_pass);
        }

        self.queue.submit(std::iter::once(label_encoder.finish()));
        Ok(())
    }

    fn render_tooltip_into_framebuffer(&mut self) -> Result<()> {
        // Determine if the price change is positive for text coloring
        let change_positive = if let (Some(vp_idx), Some(bar_idx)) =
            (self.hover_viewport_idx, self.hover_bar_index)
        {
            if let Some(state) = self.viewport_states.get(vp_idx) {
                if let Some((candles, start_idx, end_idx)) = state.get_visible_candle_range() {
                    let visible_count = end_idx.saturating_sub(start_idx);
                    if bar_idx < visible_count {
                        let candle_idx = start_idx + bar_idx;
                        if candle_idx < candles.len() {
                            candles[candle_idx].close >= candles[candle_idx].open
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                } else {
                    true
                }
            } else {
                true
            }
        } else {
            true
        };

        let mut tooltip_encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Tooltip Encoder"),
                });

        // First pass: draw background
        {
            let mut bg_pass = tooltip_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Tooltip Background Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.framebuffer_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.tooltip.draw_background(&mut bg_pass);
        }

        self.queue.submit(std::iter::once(tooltip_encoder.finish()));

        // Second pass: draw text
        let mut text_encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Tooltip Text Encoder"),
                });

        {
            let mut text_pass = text_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Tooltip Text Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.framebuffer_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.tooltip.draw_text(
                &self.device,
                &self.queue,
                &mut text_pass,
                &self.glyphon_viewport,
                change_positive,
            );
        }

        self.queue.submit(std::iter::once(text_encoder.finish()));
        Ok(())
    }

    fn copy_framebuffer_to_surface(&self, output: &wgpu::SurfaceTexture) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Framebuffer Present Encoder"),
            });

        encoder.copy_texture_to_texture(
            wgpu::ImageCopyTexture {
                texture: &self.framebuffer,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyTexture {
                texture: &output.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));
    }
}
