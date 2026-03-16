use crate::agent::state::AgentState;
use crate::market::types::{BBO, LobOrder, OrderType, Side};
use super::{GpuStepTimings, SimEngine};

pub struct CpuEngine {
    participation_threshold: f32,
    market_order_threshold: f32,
    tick_size: f32,
}

impl CpuEngine {
    pub fn new(participation_threshold: f32, market_order_threshold: f32, tick_size: f32) -> Self {
        Self { participation_threshold, market_order_threshold, tick_size }
    }
}

/// Round `price` to the nearest multiple of `tick_size`.
/// When tick_size is 0.0 the price is returned unchanged.
#[inline]
fn round_tick(price: f32, tick_size: f32) -> f32 {
    if tick_size == 0.0 {
        price
    } else {
        (price / tick_size).round() * tick_size
    }
}

impl SimEngine for CpuEngine {
    fn step(
        &mut self,
        agents: &mut AgentState,
        bbo: &BBO,
        tick: u64,
        cancel_agents: &mut Vec<u32>,
        market_orders: &mut Vec<LobOrder>,
        limit_orders: &mut Vec<LobOrder>,
    ) -> (usize, GpuStepTimings) {
        let n = agents.n;
        let k = agents.k;
        let m = agents.m;

        cancel_agents.clear();
        market_orders.clear();
        limit_orders.clear();

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

            // --- Update internal state ---
            agents.internal_state[i * m + 0] = fair_est;
            agents.internal_state[i * m + 1] = ema;
            agents.internal_state[i * m + 2] = mid;
            agents.internal_state[i * m + 3] = f32::from_bits(rng);

            // --- Classify order (inlined from convert_orders) ---
            if order_qty.abs() < f32::EPSILON {
                continue;
            }

            let is_mm = agents.agent_type[i] == 1;
            let agent_id = i as u32;

            if !is_mm
                && self.participation_threshold > 0.0
                && order_qty.abs() * aggression < self.participation_threshold
            {
                continue;
            }

            if is_mm {
                let signal_price = order_px;
                let last_mid = agents.mm_last_quote_mid[i];
                let drift = (signal_price - last_mid).abs();

                if last_mid == 0.0 || drift > agents.mm_requote_threshold[i] {
                    cancel_agents.push(agent_id);
                    let half_spread = agents.mm_half_spread[i];
                    let quote_size = agents.mm_quote_size[i];

                    limit_orders.push(LobOrder {
                        order_id: 0,
                        agent_id,
                        side: Side::Buy,
                        price: round_tick(signal_price - half_spread, self.tick_size),
                        quantity: quote_size,
                        order_type: OrderType::Limit,
                        tick: u64::MAX,
                    });
                    limit_orders.push(LobOrder {
                        order_id: 0,
                        agent_id,
                        side: Side::Sell,
                        price: round_tick(signal_price + half_spread, self.tick_size),
                        quantity: quote_size,
                        order_type: OrderType::Limit,
                        tick: u64::MAX,
                    });
                    agents.mm_last_quote_mid[i] = signal_price;
                }
            } else {
                let side = if order_qty > 0.0 { Side::Buy } else { Side::Sell };
                let qty = order_qty.abs();

                let is_market = self.market_order_threshold > 0.0
                    && qty * aggression > self.market_order_threshold;

                if is_market {
                    market_orders.push(LobOrder {
                        order_id: 0,
                        agent_id,
                        side,
                        price: order_px,
                        quantity: qty,
                        order_type: OrderType::Market,
                        tick,
                    });
                } else {
                    limit_orders.push(LobOrder {
                        order_id: 0,
                        agent_id,
                        side,
                        price: round_tick(order_px, self.tick_size),
                        quantity: qty,
                        order_type: OrderType::Limit,
                        tick,
                    });
                }
            }
        }

        (n, GpuStepTimings::default())
    }
}
