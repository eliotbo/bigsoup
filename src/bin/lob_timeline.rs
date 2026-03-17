//! Run the econsim simulation and render a depth timeline of the LOB.
//! Usage: cargo run --bin lob_timeline [-- --ticks 300 --agents 10000]

use clap::Parser;
use econsim::archetypes::Archetype;
use econsim::sim::{SimConfig, build_simulation};
use vizza::config::{ColorPalette, Theme};
use vizza::depth_timeline::{DepthTimeline, DepthTimelineEntry, DepthTimelineState};
use vizza::depth_timeline_renderer::DepthTimelineRenderer;
use vizza::depth_timeline_window::DepthTimelineWindow;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::PhysicalKey;

#[derive(Parser)]
struct Args {
    /// Number of simulation ticks to run
    #[clap(long, default_value = "10")]
    ticks: u64,

    /// Number of agents
    #[clap(long, default_value = "1000")]
    agents: usize,

    /// Per-tick volatility of the exogenous fundamental price
    #[clap(long, default_value = "0.01")]
    vol: f32,

    /// Force CPU (no GPU)
    #[clap(long)]
    cpu: bool,

    /// Random seed
    #[clap(long, default_value = "42")]
    seed: u64,

    /// Snapshot interval in ticks
    #[clap(long, default_value = "1")]
    snapshot_interval: u64,

    /// Number of LOB levels to capture per side
    #[clap(long, default_value = "50")]
    levels: usize,

    /// Output PNG path (also saves PNG before opening window)
    #[clap(long, default_value = "screenshots/lob_timeline.png")]
    output: String,

    /// Image width
    #[clap(long, default_value = "1400")]
    width: u32,

    /// Image height
    #[clap(long, default_value = "800")]
    height: u32,

    /// Full SimConfig as JSON (overrides --agents, --vol, --seed, --cpu)
    #[clap(long)]
    config: Option<String>,

    /// Skip the interactive window (just save PNG)
    #[clap(long)]
    no_window: bool,
}

struct App {
    state: DepthTimelineState,
    palette: ColorPalette,
    tick_range: (u64, u64),
    window: Option<DepthTimelineWindow>,
    cursor_x: f64,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match DepthTimelineWindow::new(event_loop, self.palette.clone(), self.tick_range) {
            Ok(mut win) => {
                win.update(&self.state);
                self.window = Some(win);
            }
            Err(e) => {
                eprintln!("Failed to create timeline window: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(win) = &mut self.window else { return };
        if win.window_id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                win.resize(size);
                // Recompute visible_count for new width
                let chart_w = size.width as f32 - 70.0;
                self.state.visible_count =
                    (chart_w / self.state.column_width_px).ceil() as usize;
                if self.state.auto_y_scale {
                    self.state.auto_scale_y();
                }
                win.update(&self.state);
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = win.render() {
                    eprintln!("Render error: {e}");
                }
            }
            WindowEvent::MouseInput { state: press, button, .. } => {
                win.on_mouse_input(button, press, self.cursor_x);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_x = position.x;
                win.on_cursor_moved(position.x, &mut self.state);
                win.update(&self.state);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 30.0,
                };
                win.on_mouse_wheel(dy, &mut self.state);
                win.update(&self.state);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        win.on_key(code, &mut self.state);
                        win.update(&self.state);
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(win) = &self.window {
            win.request_redraw();
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let config: SimConfig = if let Some(json) = &args.config {
        serde_json::from_str(json)?
    } else {
        SimConfig {
            n_agents: args.agents,
            use_gpu: Some(!args.cpu),
            seed: Some(args.seed),
            fair_value_vol: args.vol,
            init_bias: 0.02,
            market_order_threshold: 1.0,
            archetypes: Some(vec![
                Archetype {
                    name: "mean_reverter".to_string(), weight: 0.3,
                    aggression:     (0.1, 0.5),
                    mean_reversion: (0.3, 0.8),
                    trend_follow:   (0.0, 0.0),
                    noise_scale:    (0.05, 0.2),
                    ema_alpha:      (0.01, 0.1),
                    fair_value_lr:  (0.001, 0.01),
                    position_limit: (10.0, 100.0),
                    risk_aversion:  (0.01, 0.1),
                    curvature:      (0.5, 1.5),
                    midpoint:       (5.0, 20.0),
                    mm_half_spread: None, mm_quote_size: None, mm_requote_threshold: None,
                },
                Archetype {
                    name: "trend_follower".to_string(), weight: 0.3,
                    aggression:     (0.1, 0.5),
                    mean_reversion: (0.0, 0.0),
                    trend_follow:   (0.2, 0.7),
                    noise_scale:    (0.05, 0.2),
                    ema_alpha:      (0.01, 0.1),
                    fair_value_lr:  (0.001, 0.01),
                    position_limit: (10.0, 100.0),
                    risk_aversion:  (0.01, 0.1),
                    curvature:      (0.5, 1.5),
                    midpoint:       (15.0, 50.0),
                    mm_half_spread: None, mm_quote_size: None, mm_requote_threshold: None,
                },
                Archetype {
                    name: "market_maker".to_string(), weight: 0.2,
                    aggression:     (0.02, 0.1),
                    mean_reversion: (0.1, 0.3),
                    trend_follow:   (0.0, 0.0),
                    noise_scale:    (0.05, 0.2),
                    ema_alpha:      (0.01, 0.1),
                    fair_value_lr:  (0.001, 0.01),
                    position_limit: (5.0, 20.0),
                    risk_aversion:  (0.05, 0.2),
                    curvature:      (0.8, 1.2),
                    midpoint:       (3.0, 10.0),
                    mm_half_spread: Some((0.05, 0.2)),
                    mm_quote_size:  Some((1.0, 5.0)),
                    mm_requote_threshold: Some((0.05, 0.2)),
                },
                Archetype {
                    name: "noise_trader".to_string(), weight: 0.2,
                    aggression:     (0.1, 0.5),
                    mean_reversion: (0.0, 0.0),
                    trend_follow:   (0.0, 0.0),
                    noise_scale:    (0.5, 2.0),
                    ema_alpha:      (0.01, 0.1),
                    fair_value_lr:  (0.001, 0.01),
                    position_limit: (10.0, 100.0),
                    risk_aversion:  (0.01, 0.1),
                    curvature:      (0.5, 2.0),
                    midpoint:       (10.0, 50.0),
                    mm_half_spread: None, mm_quote_size: None, mm_requote_threshold: None,
                },
            ]),
            ..Default::default()
        }
    };

    let n_agents = config.n_agents;
    let mut sim = build_simulation(config)?;

    eprintln!(
        "Running {} ticks with {} agents, snapshotting LOB every {} ticks...",
        args.ticks, n_agents, args.snapshot_interval
    );

    let mut snapshots = Vec::new();

    for t in 1..=args.ticks {
        sim.step();

        if t % args.snapshot_interval == 0 {
            let bids = sim.lob.book_bids(args.levels);
            let asks = sim.lob.book_asks(args.levels);
            snapshots.push(DepthTimelineEntry {
                tick: t,
                bids,
                asks,
            });
        }
    }

    eprintln!(
        "Done. {} snapshots captured. Final price: {:.4}",
        snapshots.len(),
        sim.price_history.last().unwrap_or(&0.0)
    );

    let tick_range = (1u64, args.ticks);
    let timeline = DepthTimeline { snapshots };
    let num_snapshots = timeline.snapshots.len();

    let palette = ColorPalette::from_theme(Theme::Light);

    // Size columns to fill the chart width
    let margin_left = 70.0_f32;
    let chart_width = args.width as f32 - margin_left;
    let column_width_px = (chart_width / num_snapshots as f32).max(1.0);

    let state = DepthTimelineState::new(timeline, num_snapshots, column_width_px);

    eprintln!(
        "Price range: ${:.2} - ${:.2}, {} columns at {:.1}px each",
        state.price_min, state.price_max, num_snapshots, column_width_px
    );

    // Save PNG
    if let Some(parent) = std::path::Path::new(&args.output).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let renderer = DepthTimelineRenderer::new(args.width, args.height, palette.clone())?;
    renderer.render_to_png(&state, &args.output)?;

    // Open interactive window unless --no-window
    if args.no_window {
        return Ok(());
    }

    let event_loop = EventLoop::new()?;
    let mut app = App {
        state,
        palette,
        tick_range,
        window: None,
        cursor_x: 0.0,
    };
    event_loop.run_app(&mut app)?;

    Ok(())
}
