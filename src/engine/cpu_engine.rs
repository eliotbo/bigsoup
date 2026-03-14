use crate::agent::state::AgentState;
use crate::market::types::{BBO, Order};
use super::{GpuStepTimings, SimEngine};

pub struct CpuEngine;

impl SimEngine for CpuEngine {
    fn step(
        &mut self,
        agents: &mut AgentState,
        bbo: &BBO,
        order_buffer: &mut Vec<Order>,
    ) -> (usize, GpuStepTimings) {
        let n = agents.n;
        let k = agents.k;
        let m = agents.m;

        order_buffer.clear();
        order_buffer.reserve(n);

        for i in 0..n {
            // --- Load agent params (identical to decide.cu) ---
            let aggression     = agents.strategy_params[i * k + 0];
            let mean_reversion = agents.strategy_params[i * k + 1];
            let trend_follow   = agents.strategy_params[i * k + 2];
            let noise_scale    = agents.strategy_params[i * k + 3];
            let ema_alpha      = agents.strategy_params[i * k + 4];
            let fair_value_lr  = agents.strategy_params[i * k + 5];
            let position_limit = agents.strategy_params[i * k + 6];
            let risk_aversion  = agents.strategy_params[i * k + 7];
            let _curvature     = agents.strategy_params[i * k + 8];
            let _midpoint      = agents.strategy_params[i * k + 9];

            // --- Load internal state ---
            let mut fair_est = agents.internal_state[i * m + 0];
            let mut ema      = agents.internal_state[i * m + 1];
            let _prev_mid    = agents.internal_state[i * m + 2];
            let mut rng: u32 = f32::to_bits(agents.internal_state[i * m + 3]);

            // --- Compute ---
            let mid = (bbo.best_bid + bbo.best_ask) * 0.5;
            let spread = bbo.best_ask - bbo.best_bid;

            // Update EMA
            ema = ema + ema_alpha * (mid - ema);

            // Update fair value estimate toward exogenous fundamental (injected via bbo.fair_value)
            fair_est = fair_est + fair_value_lr * (bbo.fair_value - fair_est);

            // Signals
            let mr_signal = (fair_est - mid) * mean_reversion;
            let tf_signal = (mid - ema) * trend_follow;

            // Simple LCG noise (identical to CUDA kernel)
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let mut noise = (rng & 0xFFFF) as f32 / 65535.0 - 0.5;
            noise *= noise_scale * spread;

            // Position penalty (cast f64 position to f32 for signal math,
            // mirroring what the GPU kernel sees)
            let pos = agents.position[i] as f32;
            let pos_penalty = -risk_aversion * pos;

            // Combined signal
            let signal = mr_signal + tf_signal + noise + pos_penalty;

            // Order price and quantity
            let order_px = mid + signal * aggression;
            let mut order_qty = signal;

            // Clamp quantity by position limits
            if pos + order_qty > position_limit {
                order_qty = position_limit - pos;
            }
            if pos + order_qty < -position_limit {
                order_qty = -position_limit - pos;
            }

            // --- Write order ---
            order_buffer.push(Order {
                agent_id: i as u32,
                price: order_px,
                quantity: order_qty,
            });

            // --- Update internal state ---
            agents.internal_state[i * m + 0] = fair_est;
            agents.internal_state[i * m + 1] = ema;
            agents.internal_state[i * m + 2] = mid;
            agents.internal_state[i * m + 3] = f32::from_bits(rng);
        }

        (n, GpuStepTimings::default())
    }
}
