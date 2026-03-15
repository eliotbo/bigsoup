use rand::Rng;

#[derive(Clone)]
pub struct AgentState {
    pub n: usize,  // number of agents (fixed for lifetime of sim)
    pub k: usize,  // strategy param vector length per agent (e.g. K=8)
    pub m: usize,  // internal state vector length per agent (e.g. M=4)
    pub position: Vec<f64>,          // [N] current holdings of the asset
    pub cash: Vec<f64>,              // [N] current cash
    pub strategy_params: Vec<f32>,   // [N * K] flattened, row-major per agent
    pub internal_state: Vec<f32>,    // [N * M] flattened, row-major per agent
    /// Agent type: 0 = normal directional, 1 = market maker (two-sided quoting)
    pub agent_type: Vec<u8>,
    /// Market maker half-spread (CPU-only, not in strategy_params)
    pub mm_half_spread: Vec<f32>,
    /// Market maker quote size per side (CPU-only)
    pub mm_quote_size: Vec<f32>,
}

impl AgentState {
    pub fn new(n: usize, k: usize, m: usize) -> Self {
        Self {
            n,
            k,
            m,
            position: vec![0.0_f64; n],
            cash: vec![0.0_f64; n],
            strategy_params: vec![0.0; n * k],
            internal_state: vec![0.0; n * m],
            agent_type: vec![0; n],
            mm_half_spread: vec![0.0; n],
            mm_quote_size: vec![0.0; n],
        }
    }

    /// Fill strategy_params from per-field (min, max) uniform distributions.
    /// `distributions` is a slice of (min, max) tuples, one per param index (length K).
    pub fn randomize_params(&mut self, rng: &mut impl Rng, distributions: &[(f32, f32)]) {
        assert_eq!(distributions.len(), self.k);
        for i in 0..self.n {
            for p in 0..self.k {
                let (lo, hi) = distributions[p];
                self.strategy_params[i * self.k + p] = rng.random_range(lo..=hi);
            }
        }
    }

    pub fn get_param(&self, agent_idx: usize, param_idx: usize) -> f32 {
        self.strategy_params[agent_idx * self.k + param_idx]
    }

    pub fn set_param(&mut self, agent_idx: usize, param_idx: usize, value: f32) {
        self.strategy_params[agent_idx * self.k + param_idx] = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn test_new_allocates_zeroed() {
        let state = AgentState::new(100, 10, 4);
        assert_eq!(state.n, 100);
        assert_eq!(state.position.len(), 100);
        assert_eq!(state.cash.len(), 100);
        assert_eq!(state.strategy_params.len(), 1000);
        assert_eq!(state.internal_state.len(), 400);
        assert!(state.position.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_get_set_param() {
        let mut state = AgentState::new(10, 10, 4);
        state.set_param(3, 5, 42.0);
        assert_eq!(state.get_param(3, 5), 42.0);
        // Verify it's at the right index in the flat array
        assert_eq!(state.strategy_params[3 * 10 + 5], 42.0);
    }

    #[test]
    fn test_randomize_params() {
        let mut state = AgentState::new(1000, 10, 4);
        let mut rng = rand::rngs::StdRng::seed_from_u64(123);
        let dists = vec![
            (0.0, 1.0), (0.0, 1.0), (0.0, 1.0), (0.0, 1.0),
            (0.0, 1.0), (0.0, 1.0), (0.0, 1.0), (0.0, 1.0),
            (0.0, 1.0), (0.0, 1.0),
        ];
        state.randomize_params(&mut rng, &dists);
        // All values should be in [0, 1]
        assert!(state.strategy_params.iter().all(|&v| (0.0..=1.0).contains(&v)));
        // Not all the same (extremely unlikely with 8000 values)
        let first = state.strategy_params[0];
        assert!(state.strategy_params.iter().any(|&v| v != first));
    }
}
