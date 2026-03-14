pub mod cpu_engine;
pub mod cuda_engine;

use std::time::Duration;

use crate::agent::state::AgentState;
use crate::market::types::{BBO, Order};

/// Per-tick GPU sub-phase timings. All zeroes for CPU engine.
#[derive(Default, Clone, Copy)]
pub struct GpuStepTimings {
    pub upload: Duration,
    pub kernel: Duration,
    pub download: Duration,
}

pub trait SimEngine: Send {
    /// Run one tick: agents observe BBO, decide, emit orders.
    /// Writes into order_buffer. Returns (number of valid orders, GPU timings).
    fn step(
        &mut self,
        agents: &mut AgentState,
        bbo: &BBO,
        order_buffer: &mut Vec<Order>,
    ) -> (usize, GpuStepTimings);

    /// Recompile the kernel with a new signal expression (DSL).
    /// Only supported by CudaEngine; CpuEngine returns an error.
    fn reload_kernel(&mut self, _signal_expr: &str) -> Result<(), Box<dyn std::error::Error>> {
        Err("kernel reload not supported on this engine".into())
    }
}
