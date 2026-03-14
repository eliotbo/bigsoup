use econsim::agent::state::AgentState;
use econsim::engine::cpu_engine::CpuEngine;
use econsim::sim::{SimConfig, Simulation};
use rand::SeedableRng;

fn make_simulation(n_agents: usize, seed: u64) -> Simulation {
    let k = 8;
    let m = 4;
    let initial_price = 100.0;

    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut agents = AgentState::new(n_agents, k, m);

    for c in agents.cash.iter_mut() {
        *c = 10_000.0_f64;
    }

    for i in 0..n_agents {
        agents.internal_state[i * m + 0] = initial_price;
        agents.internal_state[i * m + 1] = initial_price;
        agents.internal_state[i * m + 2] = initial_price;
        agents.internal_state[i * m + 3] = f32::from_bits((i as u32).wrapping_mul(2654435761));
    }

    // Simple mixed params
    let dists: [(f32, f32); 8] = [
        (0.05, 0.5), (0.0, 0.5), (0.0, 0.5), (0.1, 1.0),
        (0.01, 0.1), (0.001, 0.01), (10.0, 100.0), (0.01, 0.1),
    ];
    for i in 0..n_agents {
        for p in 0..k {
            let (lo, hi) = dists[p];
            agents.strategy_params[i * k + p] = lo + (hi - lo) * rand::Rng::random::<f32>(&mut rng);
        }
    }

    let config = SimConfig {
        n_agents,
        initial_price,
        initial_cash: 10_000.0,
        k,
        m,
        use_gpu: None,
        seed: None,
        fair_value_vol: 0.0,
        init_bias: 0.0,
        archetypes: None,
    };

    Simulation::new(config, Box::new(CpuEngine), agents)
}

#[test]
fn test_smoke_1000_ticks() {
    let mut sim = make_simulation(1000, 42);
    sim.run(1000);

    assert_eq!(sim.price_history.len(), 1000);
    assert_eq!(sim.volume_history.len(), 1000);
    assert!(sim.price_history.iter().all(|p| p.is_finite()));
    assert!(sim.volume_history.iter().all(|v| v.is_finite()));
    assert!(sim.tick == 1000);
}

#[test]
fn test_deterministic() {
    let mut sim1 = make_simulation(500, 99);
    let mut sim2 = make_simulation(500, 99);
    sim1.run(100);
    sim2.run(100);

    assert_eq!(sim1.price_history, sim2.price_history);
    assert_eq!(sim1.volume_history, sim2.volume_history);
}
