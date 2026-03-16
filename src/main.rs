use econsim::agent::state::AgentState;
use econsim::archetypes::Archetype;
use econsim::engine::cpu_engine::CpuEngine;
use econsim::engine::cuda_engine::CudaEngine;
use econsim::sim::{SimConfig, Simulation};
use rand::SeedableRng;

fn main() {
    let n_ticks = 1000;

    
    let config = SimConfig {
        n_agents: 1_000_000,
        initial_price: 100.0,
        initial_cash: 10_000.0,
        k: 10,
        m: 4,
        use_gpu: Some(true),
        seed: Some(42),
        fair_value_vol: 0.002, // 0.2% per-tick random walk on fundamental
        init_bias: 0.02,       // ±2% initial fair-value disagreement between agents
        archetypes: None,      // main.rs manages archetypes directly below
        market_order_threshold: 0.0,
        participation_threshold: 0.1, // 0.1 -> 1% partcipation rate for market takers
        tick_size: 0.01,
    };

    let n_agents = config.n_agents;
    let k = config.k;
    let m = config.m;
    let initial_price = config.initial_price;

    let mut rng = rand::rngs::StdRng::seed_from_u64(config.seed.unwrap());
    let mut agents = AgentState::new(n_agents, k, m);

    // Initialize cash (f64)
    for c in agents.cash.iter_mut() {
        *c = config.initial_cash as f64;
    }

    // Initialize internal state.
    // Option 4: alternate agents start bullish / bearish by init_bias so there
    // is immediate disagreement and crossing from tick 1.
    let bias = config.init_bias;
    for i in 0..n_agents {
        let sign = if i % 2 == 0 { 1.0_f32 } else { -1.0_f32 };
        agents.internal_state[i * m + 0] = initial_price * (1.0 + sign * bias); // fair_value_estimate
        agents.internal_state[i * m + 1] = initial_price; // ema
        agents.internal_state[i * m + 2] = initial_price; // prev_mid
        agents.internal_state[i * m + 3] = f32::from_bits((i as u32).wrapping_mul(2654435761));
    }

    let archetypes = [
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
            mm_half_spread: None,
            mm_quote_size:  None,
            mm_requote_threshold: None,
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
            mm_half_spread: None,
            mm_quote_size:  None,
            mm_requote_threshold: None,
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
            mm_half_spread: None,
            mm_quote_size:  None,
            mm_requote_threshold: None,
        },
    ];

    // Assign agents to archetypes by weight, then randomize params
    let mut offset = 0;
    for archetype in &archetypes {
        let count = (archetype.weight * n_agents as f32) as usize;
        let end = (offset + count).min(n_agents);
        let dists = archetype.dists();
        for i in offset..end {
            for p in 0..k {
                let (lo, hi) = dists[p];
                agents.strategy_params[i * k + p] = lo + (hi - lo) * rand::Rng::random::<f32>(&mut rng);
            }
        }
        // Set up market maker fields
        if let Some((lo, hi)) = archetype.mm_half_spread {
            let qs = archetype.mm_quote_size.unwrap_or((1.0, 5.0));
            let rq = archetype.mm_requote_threshold.unwrap_or((0.0, 0.0));
            for i in offset..end {
                agents.agent_type[i] = 1;
                agents.mm_half_spread[i] = lo + (hi - lo) * rand::Rng::random::<f32>(&mut rng);
                agents.mm_quote_size[i] = qs.0 + (qs.1 - qs.0) * rand::Rng::random::<f32>(&mut rng);
                agents.mm_requote_threshold[i] = rq.0 + (rq.1 - rq.0) * rand::Rng::random::<f32>(&mut rng);
            }
        }
        println!("  {} agents [{}-{}): {}", archetype.name, offset, end, end - offset);
        offset = end;
    }
    // Fill any remaining agents with noise trader params
    for i in offset..n_agents {
        let dists = archetypes.last().unwrap().dists();
        for p in 0..k {
            let (lo, hi) = dists[p];
            agents.strategy_params[i * k + p] = lo + (hi - lo) * rand::Rng::random::<f32>(&mut rng);
        }
    }

    let use_gpu = config.use_gpu.unwrap_or(true);
    let engine: Box<dyn econsim::engine::SimEngine> = if use_gpu {
        match CudaEngine::new(
            0, &agents, None,
            config.participation_threshold,
            config.market_order_threshold,
            config.tick_size,
        ) {
            Ok(e) => Box::new(e),
            Err(err) => {
                eprintln!("CUDA unavailable ({err}), falling back to CPU");
                Box::new(CpuEngine::new(
                    config.participation_threshold,
                    config.market_order_threshold,
                    config.tick_size,
                ))
            }
        }
    } else {
        Box::new(CpuEngine::new(
            config.participation_threshold,
            config.market_order_threshold,
            config.tick_size,
        ))
    };
    let mut sim = Simulation::new(config, engine, agents);

    
    let engine_name = if use_gpu { "GPU engine" } else { "CPU engine" };
    println!("econsim: running {} ticks with {} agents ({})", n_ticks, n_agents, engine_name);

    let start = std::time::Instant::now();
    sim.run(n_ticks);
    let elapsed = start.elapsed();

    println!("completed in {:.2?}", elapsed);
    println!("final price: {:.4}", sim.price_history.last().unwrap());
    println!("price std:   {:.4}", std_dev(&sim.price_history));
    println!("total volume: {:.0}", sim.volume_history.iter().sum::<f32>());
    println!("price range: {:.4} - {:.4}",
        sim.price_history.iter().cloned().reduce(f32::min).unwrap(),
        sim.price_history.iter().cloned().reduce(f32::max).unwrap(),
    );

    // Check for NaN/Inf
    let bad_prices = sim.price_history.iter().filter(|p: &&f32| !p.is_finite()).count();
    let bad_volumes = sim.volume_history.iter().filter(|v: &&f32| !v.is_finite()).count();
    if bad_prices > 0 || bad_volumes > 0 {
        println!("WARNING: {} NaN/Inf prices, {} NaN/Inf volumes", bad_prices, bad_volumes);
    } else {
        println!("no NaN/Inf in outputs");
    }

    // Per-phase timing breakdown
    print_timing_table(&sim.timings, elapsed);
}

fn print_timing_table(t: &econsim::sim::StepTimings, total: std::time::Duration) {
    let total_secs = total.as_secs_f64();
    let pct = |d: std::time::Duration| -> f64 {
        if total_secs > 0.0 { d.as_secs_f64() / total_secs * 100.0 } else { 0.0 }
    };

    println!();
    println!("{:<34} {:>12}  {:>7}", "phase", "total", "%");
    println!("{}", "-".repeat(56));

    let phases: &[(&str, std::time::Duration)] = &[
        ("exo price update",              t.exo_price),
        ("gpu pipeline (total)",          t.agent_decide),
        ("  gpu upload (position)",       t.gpu_upload),
        ("  gpu kernel (decide+classify)",t.gpu_kernel),
        ("  gpu download (classified)",   t.gpu_download),
        ("order book match",              t.lob_match),
        ("fill application",              t.fill_apply),
    ];

    for (name, dur) in phases {
        println!("{:<34} {:>12.2?}  {:>6.2}%", name, dur, pct(*dur));
    }
    println!("{}", "-".repeat(56));
    println!("{:<34} {:>12.2?}  {:>6.2}%", "wall total", total, 100.0);
}

fn std_dev(data: &[f32]) -> f32 {
    let n = data.len() as f32;
    let mean = data.iter().sum::<f32>() / n;
    let variance = data.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n;
    variance.sqrt()
}
