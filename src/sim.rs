use std::time::{Duration, Instant};

use rand::SeedableRng;

use crate::agent::state::AgentState;
use crate::archetypes::Archetype;
use crate::engine::SimEngine;
use crate::market::order_book::OrderBook;
use crate::market::types::BBO;

/// Accumulated wall-clock time for each major phase across all ticks.
#[derive(Default, Clone)]
pub struct StepTimings {
    pub exo_price:    Duration,
    pub agent_decide: Duration,
    pub order_match:  Duration,
    pub fill_apply:   Duration,
    // GPU sub-phases (zero when using CPU engine)
    pub gpu_upload:   Duration,
    pub gpu_kernel:   Duration,
    pub gpu_download: Duration,
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
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            n_agents: 100_000,
            initial_price: 100.0,
            initial_cash: 10_000.0,
            k: 8,
            m: 4,
            use_gpu: Some(true),
            seed: None,
            fair_value_vol: 0.0,
            init_bias: 0.0,
            archetypes: None,
        }
    }
}

pub struct Simulation {
    pub agents: AgentState,
    pub order_book: OrderBook,
    pub engine: Box<dyn SimEngine>,
    pub tick: u64,
    pub price_history: Vec<f32>,
    pub volume_history: Vec<f32>,
    /// Current exogenous fundamental value; evolves each tick when fair_value_vol > 0.
    pub exo_price: f32,
    pub timings: StepTimings,
    fair_value_vol: f32,
    rng: rand::rngs::StdRng,
    order_buffer: Vec<crate::market::types::Order>,
}

impl Simulation {
    pub fn new(config: SimConfig, engine: Box<dyn SimEngine>, agents: AgentState) -> Self {
        let n = agents.n;
        let exo_price = config.initial_price;
        let fair_value_vol = config.fair_value_vol;
        let rng = rand::rngs::StdRng::seed_from_u64(config.seed.unwrap_or(42));
        Self {
            agents,
            order_book: OrderBook::new(config.initial_price),
            engine,
            tick: 0,
            price_history: Vec::new(),
            volume_history: Vec::new(),
            exo_price,
            timings: StepTimings::default(),
            fair_value_vol,
            rng,
            order_buffer: Vec::with_capacity(n),
        }
    }

    pub fn step(&mut self) {
        // Phase 1: Advance exogenous price process (bounded random walk)
        let t0 = Instant::now();
        if self.fair_value_vol > 0.0 {
            let u: f32 = rand::Rng::random(&mut self.rng);
            let shock = (u * 2.0 - 1.0) * self.fair_value_vol * self.exo_price;
            self.exo_price = (self.exo_price + shock).max(0.01);
        }
        self.timings.exo_price += t0.elapsed();

        // Get BBO, injecting the current fundamental value
        let mut bbo = self.order_book.bbo();
        bbo.fair_value = self.exo_price;

        // Phase 2: Engine step (agents observe + decide + emit)
        let t1 = Instant::now();
        let (_, gpu) = self.engine.step(&mut self.agents, &bbo, &mut self.order_buffer);
        self.timings.agent_decide += t1.elapsed();
        self.timings.gpu_upload   += gpu.upload;
        self.timings.gpu_kernel   += gpu.kernel;
        self.timings.gpu_download += gpu.download;

        // Phase 3: Match orders
        let t2 = Instant::now();
        let trades = self.order_book.process_orders(&self.order_buffer, self.tick);
        self.timings.order_match += t2.elapsed();

        // Phase 4: Apply fills to agent positions/cash (f64 accumulation for precision)
        let t3 = Instant::now();
        for trade in &trades {
            let qty = trade.quantity as f64;
            let px = trade.price as f64;
            self.agents.position[trade.buyer_id as usize] += qty;
            self.agents.cash[trade.buyer_id as usize] -= px * qty;
            self.agents.position[trade.seller_id as usize] -= qty;
            self.agents.cash[trade.seller_id as usize] += px * qty;
        }
        self.timings.fill_apply += t3.elapsed();

        // Record history
        let new_bbo = self.order_book.bbo();
        self.price_history.push(new_bbo.last_price);
        self.volume_history.push(trades.iter().map(|t| t.quantity.abs()).sum());

        self.tick += 1;
    }

    pub fn run(&mut self, n_ticks: u64) {
        for _ in 0..n_ticks {
            self.step();
        }
    }

    pub fn bbo(&self) -> BBO {
        self.order_book.bbo()
    }
}
