pub mod agent;
pub mod archetypes;
pub mod engine;
pub mod market;
pub mod sim;

use pyo3::prelude::*;
use numpy::IntoPyArray;

use crate::agent::state::AgentState;
use crate::engine::cpu_engine::CpuEngine;
use crate::sim::{SimConfig, Simulation};

#[pyclass]
pub struct PySimulation {
    inner: Simulation,
}

#[pymethods]
impl PySimulation {
    #[new]
    fn new(config_json: &str) -> PyResult<Self> {
        let config: SimConfig = serde_json::from_str(config_json)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;

        let n = config.n_agents;
        let k = config.k;
        let m = config.m;
        let initial_price = config.initial_price;
        let seed = config.seed.unwrap_or(42);

        let mut agents = AgentState::new(n, k, m);
        for c in agents.cash.iter_mut() {
            *c = config.initial_cash as f64;
        }
        let bias = config.init_bias;
        for i in 0..n {
            let sign = if i % 2 == 0 { 1.0_f32 } else { -1.0_f32 };
            agents.internal_state[i * m + 0] = initial_price * (1.0 + sign * bias);
            agents.internal_state[i * m + 1] = initial_price;
            agents.internal_state[i * m + 2] = initial_price;
            agents.internal_state[i * m + 3] = f32::from_bits((i as u32).wrapping_mul(2654435761));
        }

        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

        if let Some(archetypes) = &config.archetypes {
            // Partition agents by archetype weight and draw params from each archetype's ranges.
            let mut offset = 0;
            for archetype in archetypes {
                let count = (archetype.weight * n as f32) as usize;
                let end = (offset + count).min(n);
                let dists = archetype.dists();
                for i in offset..end {
                    for p in 0..k {
                        let (lo, hi) = dists[p];
                        agents.strategy_params[i * k + p] = rand::Rng::random_range(&mut rng, lo..=hi);
                    }
                }
                offset = end;
            }
            // Fill any remaining agents with the last archetype's ranges.
            if offset < n {
                if let Some(last) = archetypes.last() {
                    let dists = last.dists();
                    for i in offset..n {
                        for p in 0..k {
                            let (lo, hi) = dists[p];
                            agents.strategy_params[i * k + p] = rand::Rng::random_range(&mut rng, lo..=hi);
                        }
                    }
                }
            }
        } else {
            let dists = default_param_distributions();
            agents.randomize_params(&mut rng, &dists);
        }

        let use_gpu = config.use_gpu.unwrap_or(true);
        let engine: Box<dyn crate::engine::SimEngine> = if use_gpu {
            let cuda_engine = crate::engine::cuda_engine::CudaEngine::new(0, &agents)
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
            Box::new(cuda_engine)
        } else {
            Box::new(CpuEngine)
        };

        let sim = Simulation::new(config, engine, agents);
        Ok(Self { inner: sim })
    }

    fn step(&mut self) {
        self.inner.step();
    }

    fn run(&mut self, n_ticks: u64) {
        self.inner.run(n_ticks);
    }

    fn price_history<'py>(&self, py: Python<'py>) -> Bound<'py, numpy::PyArray1<f32>> {
        self.inner.price_history.clone().into_pyarray_bound(py)
    }

    fn volume_history<'py>(&self, py: Python<'py>) -> Bound<'py, numpy::PyArray1<f32>> {
        self.inner.volume_history.clone().into_pyarray_bound(py)
    }

    fn agent_positions<'py>(&self, py: Python<'py>) -> Bound<'py, numpy::PyArray1<f64>> {
        self.inner.agents.position.clone().into_pyarray_bound(py)
    }

    fn agent_cash<'py>(&self, py: Python<'py>) -> Bound<'py, numpy::PyArray1<f64>> {
        self.inner.agents.cash.clone().into_pyarray_bound(py)
    }

    fn bbo(&self) -> (f32, f32, f32, f32, f32) {
        let b = self.inner.bbo();
        (b.best_bid, b.best_bid_size, b.best_ask, b.best_ask_size, b.last_price)
    }

    fn tick(&self) -> u64 {
        self.inner.tick
    }
}

fn default_param_distributions() -> Vec<(f32, f32)> {
    // noise_scale must be large enough that signal * aggression exceeds the initial spread
    vec![
        (0.1, 0.5),    // aggression
        (0.0, 0.5),    // mean_reversion
        (0.0, 0.5),    // trend_follow
        (0.5, 2.0),    // noise_scale  (match main.rs noise_trader range so orders cross)
        (0.01, 0.2),   // ema_alpha
        (0.001, 0.01), // fair_value_lr
        (10.0, 100.0), // position_limit
        (0.01, 0.1),   // risk_aversion
    ]
}

#[pymodule]
fn econsim(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySimulation>()?;
    Ok(())
}
