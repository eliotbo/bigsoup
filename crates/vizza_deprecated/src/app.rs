use anyhow::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

use crate::{Config, MarketData, event::MouseState, live::LiveDataSource, renderer::Renderer, depth_window::DepthWindow, depth_snapshot::DepthSnapshot};

pub struct ChartApp {
    config: Config,
    use_live_data: bool,
    custom_live_source: Option<Box<dyn LiveDataSource>>,
    custom_live_sources: Vec<(String, Box<dyn LiveDataSource>)>,
    today_so_far_enabled: bool,
    preloaded_market_data: Option<MarketData>,
    per_viewport_market_data: Option<Vec<MarketData>>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    mouse_state: MouseState,
    ctrl_pressed: bool,
    live_tick_interval: Option<Duration>,
    next_live_tick: Option<Instant>,
    needs_redraw: bool,
    // Depth order book window
    depth_window: Option<DepthWindow>,
    depth_enabled: bool,
    pending_depth_snapshot: Option<DepthSnapshot>,
}

impl ChartApp {
    fn make_live_tick_interval(
        use_live_data: bool,
        live_update_interval_ms: Option<u64>,
    ) -> Option<Duration> {
        if !use_live_data {
            return None;
        }

        match live_update_interval_ms {
            Some(ms) if ms > 0 => Some(Duration::from_millis(ms)),
            Some(_) => Some(Duration::from_millis(1)),
            None => Some(Duration::from_millis(16)),
        }
    }

    pub fn new(
        config: Config,
        use_live_data: bool,
        live_update_interval_ms: Option<u64>,
        preloaded_market_data: Option<MarketData>,
        per_viewport_market_data: Option<Vec<MarketData>>,
    ) -> Self {
        Self {
            config,
            use_live_data,
            custom_live_source: None,
            custom_live_sources: Vec::new(),
            today_so_far_enabled: false,
            preloaded_market_data,
            per_viewport_market_data,
            window: None,
            renderer: None,
            mouse_state: MouseState::new(),
            ctrl_pressed: false,
            live_tick_interval: Self::make_live_tick_interval(
                use_live_data,
                live_update_interval_ms,
            ),
            next_live_tick: None,
            needs_redraw: false,
            depth_window: None,
            depth_enabled: false,
            pending_depth_snapshot: None,
        }
    }

    pub fn with_custom_live_source(
        config: Config,
        use_live_data: bool,
        live_update_interval_ms: Option<u64>,
        custom_live_source: Option<Box<dyn LiveDataSource>>,
        today_so_far_enabled: bool,
        preloaded_market_data: Option<MarketData>,
        per_viewport_market_data: Option<Vec<MarketData>>,
    ) -> Self {
        Self {
            config,
            use_live_data,
            custom_live_source,
            custom_live_sources: Vec::new(),
            today_so_far_enabled,
            preloaded_market_data,
            per_viewport_market_data,
            window: None,
            renderer: None,
            mouse_state: MouseState::new(),
            ctrl_pressed: false,
            live_tick_interval: Self::make_live_tick_interval(
                use_live_data,
                live_update_interval_ms,
            ),
            next_live_tick: None,
            needs_redraw: false,
            depth_window: None,
            depth_enabled: false,
            pending_depth_snapshot: None,
        }
    }

    pub fn with_custom_live_sources(
        config: Config,
        use_live_data: bool,
        live_update_interval_ms: Option<u64>,
        custom_live_sources: Vec<(String, Box<dyn LiveDataSource>)>,
        today_so_far_enabled: bool,
        preloaded_market_data: Option<MarketData>,
        per_viewport_market_data: Option<Vec<MarketData>>,
    ) -> Self {
        Self {
            config,
            use_live_data,
            custom_live_source: None,
            custom_live_sources,
            today_so_far_enabled,
            preloaded_market_data,
            per_viewport_market_data,
            window: None,
            renderer: None,
            mouse_state: MouseState::new(),
            ctrl_pressed: false,
            live_tick_interval: Self::make_live_tick_interval(
                use_live_data,
                live_update_interval_ms,
            ),
            next_live_tick: None,
            needs_redraw: false,
            depth_window: None,
            depth_enabled: false,
            pending_depth_snapshot: None,
        }
    }

    /// Enable the depth order book window. Must be called before running the event loop.
    pub fn enable_depth_window(&mut self) {
        self.depth_enabled = true;
    }

    /// Submit a new depth snapshot to be rendered in the depth window.
    pub fn submit_depth_snapshot(&mut self, snapshot: DepthSnapshot) {
        if let Some(dw) = &mut self.depth_window {
            dw.update_snapshot(&snapshot);
            dw.request_redraw();
        } else {
            // Store for when the window is created
            self.pending_depth_snapshot = Some(snapshot);
        }
    }

    fn mark_dirty(&mut self) {
        if self.needs_redraw || self.window.is_none() {
            return;
        }
        self.needs_redraw = true;
        self.mouse_state.redraw_pending = true;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler for ChartApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window_attributes = Window::default_attributes()
                .with_title("Vizza Chart")
                .with_inner_size(winit::dpi::LogicalSize::new(
                    self.config.window_width,
                    self.config.window_height,
                ));

            let preloaded_market_data = self.preloaded_market_data.take();
            let per_viewport_market_data = self.per_viewport_market_data.take();

            match event_loop.create_window(window_attributes) {
                Ok(window) => {
                    let window = Arc::new(window);
                    let renderer = if !self.custom_live_sources.is_empty() {
                        // Multiple sources path
                        let sources = std::mem::take(&mut self.custom_live_sources);
                        pollster::block_on(Renderer::new_with_sources(
                            Arc::clone(&window),
                            self.config.clone(),
                            sources,
                            self.use_live_data,
                            preloaded_market_data,
                            per_viewport_market_data,
                            self.today_so_far_enabled,
                        ))
                    } else {
                        // Single source path (backward compatibility)
                        pollster::block_on(Renderer::new(
                            Arc::clone(&window),
                            self.config.clone(),
                            self.custom_live_source.take(),
                            self.use_live_data,
                            preloaded_market_data,
                            per_viewport_market_data,
                            self.today_so_far_enabled,
                        ))
                    };
                    match renderer {
                        Ok(renderer) => {
                            self.window = Some(window);
                            self.renderer = Some(renderer);
                            if self.live_tick_interval.is_some() {
                                self.next_live_tick = Some(Instant::now());
                            }
                            self.mark_dirty();

                            // Create depth window if enabled
                            if self.depth_enabled && self.depth_window.is_none() {
                                let palette = crate::config::ColorPalette::from_theme(self.config.theme);
                                match DepthWindow::new(event_loop, palette) {
                                    Ok(mut dw) => {
                                        if let Some(snap) = self.pending_depth_snapshot.take() {
                                            dw.update_snapshot(&snap);
                                        }
                                        self.depth_window = Some(dw);
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to create depth window: {}", e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to create renderer: {}", e);
                            event_loop.exit();
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to create window: {}", e);
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Check if the event belongs to the depth window
        if let Some(dw) = &mut self.depth_window {
            if window_id == dw.window_id() {
                match event {
                    WindowEvent::RedrawRequested => {
                        if let Err(e) = dw.render() {
                            eprintln!("Depth render error: {}", e);
                        }
                    }
                    WindowEvent::Resized(new_size) => {
                        dw.resize(new_size);
                        dw.request_redraw();
                    }
                    WindowEvent::CloseRequested => {
                        self.depth_window = None;
                    }
                    _ => {}
                }
                return;
            }
        }

        match event {
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_mut() {
                    if let Err(e) = renderer.render() {
                        eprintln!("Render error: {}", e);
                    }
                }
                self.mouse_state.redraw_pending = false;
                self.needs_redraw = false;
            }
            other => {
                let dirty = crate::event::dispatch_window_event(
                    other,
                    event_loop,
                    &mut self.renderer,
                    &mut self.mouse_state,
                    &mut self.ctrl_pressed,
                );

                if dirty {
                    self.mark_dirty();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let mut next_deadline = None;

        if let Some(interval) = self.live_tick_interval {
            let next_tick = self.next_live_tick.unwrap_or(now);

            if next_tick <= now {
                if let Some(renderer) = self.renderer.as_mut() {
                    if renderer.tick_live() {
                        self.mark_dirty();
                    }
                }
                let scheduled = now + interval;
                self.next_live_tick = Some(scheduled);
                next_deadline = Some(scheduled);
            } else {
                next_deadline = Some(next_tick);
            }
        }

        if self.needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            if let Some(dw) = &self.depth_window {
                dw.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::Wait);
        } else if let Some(deadline) = next_deadline {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// Run the vizza application with the given configuration
pub fn run_with_config(
    config: Config,
    use_live_data: bool,
    live_update_interval_ms: Option<u64>,
    preloaded_market_data: Option<MarketData>,
    per_viewport_market_data: Option<Vec<MarketData>>,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = ChartApp::new(
        config,
        use_live_data,
        live_update_interval_ms,
        preloaded_market_data,
        per_viewport_market_data,
    );

    event_loop.run_app(&mut app)?;

    Ok(())
}

/// Run the vizza application with a custom live data source
pub fn run_with_config_and_source(
    config: Config,
    use_live_data: bool,
    live_update_interval_ms: Option<u64>,
    custom_live_source: Option<Box<dyn LiveDataSource>>,
    today_so_far_enabled: bool,
    preloaded_market_data: Option<MarketData>,
    per_viewport_market_data: Option<Vec<MarketData>>,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = ChartApp::with_custom_live_source(
        config,
        use_live_data,
        live_update_interval_ms,
        custom_live_source,
        today_so_far_enabled,
        preloaded_market_data,
        per_viewport_market_data,
    );

    event_loop.run_app(&mut app)?;

    Ok(())
}

/// Run the vizza application with multiple custom live data sources (one per viewport)
pub fn run_with_config_and_sources(
    config: Config,
    use_live_data: bool,
    live_update_interval_ms: Option<u64>,
    custom_live_sources: Vec<(String, Box<dyn LiveDataSource>)>,
    today_so_far_enabled: bool,
    preloaded_market_data: Option<MarketData>,
    per_viewport_market_data: Option<Vec<MarketData>>,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = ChartApp::with_custom_live_sources(
        config,
        use_live_data,
        live_update_interval_ms,
        custom_live_sources,
        today_so_far_enabled,
        preloaded_market_data,
        per_viewport_market_data,
    );

    event_loop.run_app(&mut app)?;

    Ok(())
}
