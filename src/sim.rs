use std::time::{Duration, Instant};

use rand::SeedableRng;

use crate::agent::state::AgentState;
use crate::archetypes::Archetype;
use crate::engine::SimEngine;
use crate::market::lob::LimitOrderBook;
use crate::market::types::{BBO, LobOrder, OrderType, Side};

/// Build a `Simulation` from a `SimConfig`, initialising agents and selecting
/// the GPU or CPU engine automatically.
pub fn build_simulation(config: SimConfig) -> anyhow::Result<Simulation> {
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

    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    if let Some(archetypes) = &config.archetypes {
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
            // Set up market maker fields if this archetype has them
            if let Some((lo, hi)) = archetype.mm_half_spread {
                let qs = archetype.mm_quote_size.unwrap_or((1.0, 5.0));
                for i in offset..end {
                    agents.agent_type[i] = 1;
                    agents.mm_half_spread[i] = rand::Rng::random_range(&mut rng, lo..=hi);
                    agents.mm_quote_size[i] = rand::Rng::random_range(&mut rng, qs.0..=qs.1);
                }
            }
            offset = end;
        }
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
        // Uniform defaults
        let dists: Vec<(f32, f32)> = vec![
            (0.1, 0.5), (0.0, 0.5), (0.0, 0.5), (0.5, 2.0),
            (0.01, 0.2), (0.001, 0.01), (10.0, 100.0), (0.01, 0.1),
            (0.5, 2.0), (5.0, 50.0),
        ];
        agents.randomize_params(&mut rng, &dists);
    }

    let use_gpu = config.use_gpu.unwrap_or(true);
    let engine: Box<dyn SimEngine> = if use_gpu {
        match crate::engine::cuda_engine::CudaEngine::new(0, &agents, None) {
            Ok(e) => Box::new(e),
            Err(err) => {
                eprintln!("CUDA unavailable ({err}), falling back to CPU");
                Box::new(crate::engine::cpu_engine::CpuEngine)
            }
        }
    } else {
        Box::new(crate::engine::cpu_engine::CpuEngine)
    };

    Ok(Simulation::new(config, engine, agents))
}

/// Accumulated wall-clock time for each major phase across all ticks.
#[derive(Default, Clone)]
pub struct StepTimings {
    pub exo_price:      Duration,
    pub agent_decide:   Duration,
    pub order_convert:  Duration,
    pub lob_match:      Duration,
    pub fill_apply:     Duration,
    pub lob_expire:     Duration,
    // GPU sub-phases (zero when using CPU engine)
    pub gpu_upload:     Duration,
    pub gpu_kernel:     Duration,
    pub gpu_download:   Duration,
}

#[derive(serde::Deserialize)]
pub struct SimConfig {
    pub n_agents: usize,
    pub initial_price: f32,
    pub initial_cash: f32,
    pub k: usize,
    pub m: usize,
    #[serde(default)]
    pub use_gpu: Option<bool>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Per-tick volatility of the exogenous fundamental price as a fraction of
    /// current price.  0.0 = no exogenous process (default).  Try ~0.002.
    #[serde(default)]
    pub fair_value_vol: f32,
    /// Initial disagreement: agent i starts with fair_value_estimate =
    /// initial_price * (1 ± init_bias) alternating bullish/bearish.
    /// 0.0 = all start at initial_price (default).  Try ~0.02.
    #[serde(default)]
    pub init_bias: f32,
    /// Agent archetypes with weights and per-parameter ranges.
    /// When provided, agents are partitioned by weight and each group gets
    /// params drawn from that archetype's ranges.
    /// When absent, a single uniform distribution over all agents is used.
    #[serde(default)]
    pub archetypes: Option<Vec<Archetype>>,
    /// Minimum |signal| * aggression to submit a market order instead of limit.
    /// 0.0 = disabled (all orders are limit orders).
    #[serde(default)]
    pub market_order_threshold: f32,
    /// Minimum |quantity| * aggression for a non-MM agent to participate at all.
    /// 0.0 = everyone participates every tick (default, backward compatible).
    #[serde(default)]
    pub participation_threshold: f32,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            n_agents: 100_000,
            initial_price: 100.0,
            initial_cash: 10_000.0,
            k: 10,
            m: 4,
            use_gpu: Some(true),
            seed: None,
            fair_value_vol: 0.0,
            init_bias: 0.0,
            archetypes: None,
            market_order_threshold: 0.0,
            participation_threshold: 0.0,
        }
    }
}

pub struct Simulation {
    pub agents: AgentState,
    pub lob: LimitOrderBook,
    pub engine: Box<dyn SimEngine>,
    pub tick: u64,
    pub price_history: Vec<f32>,
    pub volume_history: Vec<f32>,
    pub exo_price_history: Vec<f32>,
    pub spread_history: Vec<f32>,
    pub bid_depth_history: Vec<f32>,
    pub ask_depth_history: Vec<f32>,
    /// Current exogenous fundamental value; evolves each tick when fair_value_vol > 0.
    pub exo_price: f32,
    pub timings: StepTimings,
    fair_value_vol: f32,
    market_order_threshold: f32,
    participation_threshold: f32,
    rng: rand::rngs::StdRng,
    order_buffer: Vec<crate::market::types::Order>,
}

impl Simulation {
    pub fn new(config: SimConfig, engine: Box<dyn SimEngine>, agents: AgentState) -> Self {
        let n = agents.n;
        let exo_price = config.initial_price;
        let fair_value_vol = config.fair_value_vol;
        let market_order_threshold = config.market_order_threshold;
        let participation_threshold = config.participation_threshold;
        let rng = rand::rngs::StdRng::seed_from_u64(config.seed.unwrap_or(42));
        Self {
            agents,
            lob: LimitOrderBook::new(config.initial_price),
            engine,
            tick: 0,
            price_history: Vec::new(),
            volume_history: Vec::new(),
            exo_price_history: Vec::new(),
            spread_history: Vec::new(),
            bid_depth_history: Vec::new(),
            ask_depth_history: Vec::new(),
            exo_price,
            timings: StepTimings::default(),
            fair_value_vol,
            market_order_threshold,
            participation_threshold,
            rng,
            order_buffer: Vec::with_capacity(n),
        }
    }

    pub fn step(&mut self) {
        // Phase 1: Advance exogenous price process (bounded random walk)
        let t0 = Instant::now();
        if self.fair_value_vol > 0.0 {
            let u: f32 = rand::Rng::random(&mut self.rng);
            let log_shock = (u * 2.0 - 1.0) * self.fair_value_vol;
            self.exo_price = (self.exo_price.ln() + log_shock).exp();
        }
        self.timings.exo_price += t0.elapsed();

        // BBO from LOB (reflects standing resting orders)
        let mut bbo = self.lob.bbo();
        bbo.fair_value = self.exo_price;

        // Phase 2: Engine step (agents observe + decide + emit)
        let t1 = Instant::now();
        let (_, gpu) = self.engine.step(&mut self.agents, &bbo, &mut self.order_buffer);
        self.timings.agent_decide += t1.elapsed();
        self.timings.gpu_upload   += gpu.upload;
        self.timings.gpu_kernel   += gpu.kernel;
        self.timings.gpu_download += gpu.download;

        // Phase 3: Convert kernel orders to LOB orders
        let t2 = Instant::now();
        let (cancel_agents, market_orders, limit_orders) = convert_orders(
            &self.order_buffer,
            &self.agents,
            self.tick,
            self.market_order_threshold,
            self.participation_threshold,
        );
        self.timings.order_convert += t2.elapsed();

        // Phase 4: Process tick on LOB (cancel, match, rest)
        let t3 = Instant::now();
        let trades = self.lob.process_tick(&cancel_agents, market_orders, limit_orders, self.tick);
        self.timings.lob_match += t3.elapsed();

        // Phase 5: Apply fills to agent positions/cash (f64 accumulation for precision)
        let t4 = Instant::now();
        for trade in &trades {
            let qty = trade.quantity as f64;
            let px = trade.price as f64;
            self.agents.position[trade.buyer_id as usize] += qty;
            self.agents.cash[trade.buyer_id as usize] -= px * qty;
            self.agents.position[trade.seller_id as usize] -= qty;
            self.agents.cash[trade.seller_id as usize] += px * qty;
        }
        self.timings.fill_apply += t4.elapsed();

        // Phase 6: Expire stale orders (1-tick TTL)
        let t5 = Instant::now();
        self.lob.expire_orders_before(self.tick);
        self.timings.lob_expire += t5.elapsed();

        // Record history
        let new_bbo = self.lob.bbo();
        self.price_history.push(new_bbo.last_price);
        self.volume_history.push(trades.iter().map(|t| t.quantity.abs()).sum());
        self.exo_price_history.push(self.exo_price);
        self.spread_history.push(new_bbo.best_ask - new_bbo.best_bid);
        self.bid_depth_history.push(self.lob.bids_total_qty());
        self.ask_depth_history.push(self.lob.asks_total_qty());

        self.tick += 1;
    }

    pub fn run(&mut self, n_ticks: u64) {
        for _ in 0..n_ticks {
            self.step();
        }
    }

    pub fn bbo(&self) -> BBO {
        let mut bbo = self.lob.bbo();
        bbo.fair_value = self.exo_price;
        bbo
    }
}

/// Convert kernel output orders into LOB orders, handling market maker
/// two-sided quoting and market order promotion.
fn convert_orders(
    order_buffer: &[crate::market::types::Order],
    agents: &AgentState,
    tick: u64,
    market_order_threshold: f32,
    participation_threshold: f32,
) -> (Vec<u32>, Vec<LobOrder>, Vec<LobOrder>) {
    let mut cancel_agents = Vec::new();
    let mut market_orders = Vec::new();
    let mut limit_orders = Vec::new();

    for order in order_buffer {
        if order.quantity.abs() < f32::EPSILON {
            continue;
        }

        let i = order.agent_id as usize;
        let is_mm = agents.agent_type[i] == 1;

        // Non-MM agents with weak signals sit out entirely
        if !is_mm
            && participation_threshold > 0.0
            && order.quantity.abs() * agents.get_param(i, 0) < participation_threshold
        {
            continue;
        }

        if is_mm {
            // Market maker: cancel old quotes, post two-sided
            cancel_agents.push(order.agent_id);
            let half_spread = agents.mm_half_spread[i];
            let quote_size = agents.mm_quote_size[i];
            let signal_price = order.price;

            limit_orders.push(LobOrder {
                order_id: 0,
                agent_id: order.agent_id,
                side: Side::Buy,
                price: signal_price - half_spread,
                quantity: quote_size,
                order_type: OrderType::Limit,
                tick,
            });
            limit_orders.push(LobOrder {
                order_id: 0,
                agent_id: order.agent_id,
                side: Side::Sell,
                price: signal_price + half_spread,
                quantity: quote_size,
                order_type: OrderType::Limit,
                tick,
            });
        } else {
            let side = if order.quantity > 0.0 {
                Side::Buy
            } else {
                Side::Sell
            };
            let qty = order.quantity.abs();

            // Promote to market order if signal is strong enough
            let aggression = agents.get_param(i, 0);
            let is_market = market_order_threshold > 0.0
                && qty * aggression > market_order_threshold;

            if is_market {
                market_orders.push(LobOrder {
                    order_id: 0,
                    agent_id: order.agent_id,
                    side,
                    price: order.price,
                    quantity: qty,
                    order_type: OrderType::Market,
                    tick,
                });
            } else {
                limit_orders.push(LobOrder {
                    order_id: 0,
                    agent_id: order.agent_id,
                    side,
                    price: order.price,
                    quantity: qty,
                    order_type: OrderType::Limit,
                    tick,
                });
            }
        }
    }

    (cancel_agents, market_orders, limit_orders)
}
