/// Named parameter ranges for one agent archetype.
/// Each field is a `(min, max)` uniform distribution for that strategy parameter.
///
/// Param order in the flat strategy_params array (K=10):
///   [aggression, mean_reversion, trend_follow, noise_scale,
///    ema_alpha, fair_value_lr, position_limit, risk_aversion, curvature, midpoint]
#[derive(serde::Deserialize, Clone)]
pub struct Archetype {
    pub name: String,
    /// Fraction of total agents assigned to this archetype (should sum to 1.0).
    pub weight: f32,
    pub aggression:     (f32, f32),
    pub mean_reversion: (f32, f32),
    pub trend_follow:   (f32, f32),
    pub noise_scale:    (f32, f32),
    pub ema_alpha:      (f32, f32),
    pub fair_value_lr:  (f32, f32),
    pub position_limit: (f32, f32),
    pub risk_aversion:  (f32, f32),
    pub curvature:      (f32, f32),
    pub midpoint:       (f32, f32),
    /// Market maker half-spread range (min, max). When set, agents use two-sided quoting.
    #[serde(default)]
    pub mm_half_spread: Option<(f32, f32)>,
    /// Market maker quote size range (min, max) per side.
    #[serde(default)]
    pub mm_quote_size:  Option<(f32, f32)>,
    /// MM requote threshold range (min, max). MM only cancels+requotes when
    /// |new_signal - last_quote_mid| > threshold. None = always requote (0.0).
    #[serde(default)]
    pub mm_requote_threshold: Option<(f32, f32)>,
}

impl Archetype {
    /// Returns param distributions in the flat K=10 order expected by `AgentState::randomize_params`.
    pub fn dists(&self) -> [(f32, f32); 10] {
        [
            self.aggression,
            self.mean_reversion,
            self.trend_follow,
            self.noise_scale,
            self.ema_alpha,
            self.fair_value_lr,
            self.position_limit,
            self.risk_aversion,
            self.curvature,
            self.midpoint,
        ]
    }
}

impl<'a> From<&'a Archetype> for (&'a str, f32, [(f32, f32); 10]) {
    fn from(a: &'a Archetype) -> Self {
        (a.name.as_str(), a.weight, a.dists())
    }
}
