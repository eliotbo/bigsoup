pub mod agent;
pub mod archetypes;
pub mod engine;
pub mod market;
pub mod sim;

use pyo3::prelude::*;
use numpy::IntoPyArray;

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
        let sim = crate::sim::build_simulation(config)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
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

    /// Recompile and hot-swap the CUDA kernel with a new signal expression.
    /// The expression should be a C float expression string produced by
    /// `econsim.dsl.compile()`.
    fn set_strategy(&mut self, signal_expr: &str) -> PyResult<()> {
        self.inner.engine.reload_kernel(signal_expr)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
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
        (0.5, 2.0),    // curvature
        (5.0, 50.0),   // midpoint
    ]
}

#[pymodule]
fn econsim(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySimulation>()?;
    Ok(())
}
