use clap::Parser;
use econsim::sim::{SimConfig, build_simulation};
use lod::{LevelStore, PlotCandle};
use vizza::{LineOverlay, LodLevel, MarketData, PlotBuilder};

/*
cargo install --path . --bin chart
*/

#[derive(Parser)]
struct Args {
    /// Number of simulation ticks to run
    #[clap(long, default_value = "2000")]
    ticks: u64,

    /// Number of agents
    #[clap(long, default_value = "10000")]
    agents: usize,

    /// Per-tick volatility of the exogenous fundamental price (higher = faster price moves)
    #[clap(long, default_value = "0.01")]
    vol: f32,

    /// Force CPU (no GPU)
    #[clap(long)]
    cpu: bool,

    /// Random seed
    #[clap(long, default_value = "42")]
    seed: u64,

    /// DSL strategy expression (compiled C float expression from econsim DSL)
    #[clap(long)]
    strategy: Option<String>,

    /// Full SimConfig as JSON (overrides --agents, --vol, --seed, --cpu)
    #[clap(long)]
    config: Option<String>,
}

fn aggregate_candles(
    prices: &[f32],
    volumes: &[f32],
    ticks_per_candle: usize,
    base_ts_ns: i64,
    tick_duration_ns: i64,
) -> Vec<PlotCandle> {
    prices
        .chunks(ticks_per_candle)
        .enumerate()
        .filter_map(|(i, chunk)| {
            let vols = &volumes[i * ticks_per_candle..(i * ticks_per_candle + chunk.len())];
            let open = chunk[0];
            let close = chunk[chunk.len() - 1];
            let high = chunk.iter().cloned().reduce(f32::max)?;
            let low = chunk.iter().cloned().reduce(f32::min)?;
            let volume: f32 = vols.iter().sum();
            let ts = base_ts_ns + i as i64 * ticks_per_candle as i64 * tick_duration_ns;
            Some(PlotCandle::new(ts, open, high, low, close, volume))
        })
        .collect()
}

fn downsample_line(
    values: &[f32],
    ticks_per_point: usize,
    base_ts_ns: i64,
    tick_ns: i64,
) -> Vec<(i64, f32)> {
    values
        .chunks(ticks_per_point)
        .enumerate()
        .map(|(i, chunk)| {
            let ts = base_ts_ns + i as i64 * ticks_per_point as i64 * tick_ns;
            (ts, *chunk.last().unwrap())
        })
        .collect()
}

fn build_level_store(
    prices: &[f32],
    volumes: &[f32],
    base_ts_ns: i64,
    tick_ns: i64,
) -> LevelStore {
    // LevelStore keys must exactly match LodLevel::seconds() values.
    // Each simulation tick = tick_ns nanoseconds (1 second by default).
    // ticks_per_candle = lod_seconds / tick_seconds.
    let interval_ns = |secs: u64| secs as i64 * 1_000_000_000;

    // All LodLevel second values, finest to coarsest
    let lod_intervals: &[u64] = &[1, 5, 15, 30, 60, 300, 900, 1800, 3600, 14400, 86400];

    let mut levels: Vec<(u64, Vec<PlotCandle>)> = Vec::new();
    for &interval_secs in lod_intervals {
        let ticks_per_candle = (interval_ns(interval_secs) / tick_ns).max(1) as usize;
        if ticks_per_candle > prices.len() {
            break; // coarser than all our data
        }
        let candles = aggregate_candles(prices, volumes, ticks_per_candle, base_ts_ns, tick_ns);
        if candles.len() >= 2 {
            levels.push((interval_secs, candles));
        }
    }

    LevelStore::from_stream(levels)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let config: SimConfig = if let Some(json) = &args.config {
        serde_json::from_str(json)?
    } else {
        SimConfig {
            n_agents: args.agents,
            initial_price: 100.0,
            initial_cash: 10_000.0,
            k: 10,
            m: 4,
            use_gpu: Some(!args.cpu),
            seed: Some(args.seed),
            fair_value_vol: args.vol,
            init_bias: 0.02,
            archetypes: None,
        }
    };

    let n_agents = config.n_agents;
    let mut sim = build_simulation(config)?;

    if let Some(strategy) = &args.strategy {
        if let Err(e) = sim.engine.reload_kernel(strategy) {
            eprintln!("Warning: failed to load strategy: {e}");
        }
    }

    eprintln!("Running {} ticks with {} agents...", args.ticks, n_agents);
    sim.run(args.ticks);
    eprintln!("Done. Final price: {:.4}", sim.price_history.last().unwrap_or(&0.0));

    // Use timestamps anchored to "now - n_ticks seconds" so the chart shows
    // the simulation as if it just happened.
    let now_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let tick_ns: i64 = 10_000_000; // 10ms per tick
    let base_ts_ns = now_ns - args.ticks as i64 * tick_ns;

    let level_store = build_level_store(
        &sim.price_history,
        &sim.volume_history,
        base_ts_ns,
        tick_ns,
    );

    let market_data = MarketData::from_level_store(level_store);

    let ticks_per_second = (1_000_000_000 / tick_ns).max(1) as usize;
    let exo_overlay = LineOverlay::new(
        downsample_line(&sim.exo_price_history, ticks_per_second, base_ts_ns, tick_ns),
        [0.4, 0.7, 1.0, 0.5], // light blue, semi-transparent
    );

    PlotBuilder::new()
        .with_window_size(1400, 700)
        .with_market_data(market_data)
        .with_lod_level(LodLevel::S1)
        .with_line_overlays(vec![vec![exo_overlay]])
        .with_title("econsim")
        .run()?;

    Ok(())
}
