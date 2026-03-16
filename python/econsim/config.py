from dataclasses import dataclass, field
from typing import Optional
import json


@dataclass
class ArchetypeConfig:
    """Parameter ranges for one agent archetype.

    Each *_range is a (min, max) tuple used to draw that strategy parameter
    from a uniform distribution.  Param order matches K=10 layout:
      aggression, mean_reversion, trend_follow, noise_scale,
      ema_alpha, fair_value_lr, position_limit, risk_aversion, curvature, midpoint


    The desirability curve is defined as:
        raw_signal * (1.0f / (1.0f + expf((curvature * (fabsf(pos) - midpoint)))))

        When |pos| < midpoint, the exponent is negative, sigmoid is near 1 — full signal. When |pos| >
        midpoint, sigmoid drops toward 0 — agent loses interest. curvature controls sharpness of the
        transition. Each archetype has its own ranges:

        
        ┌────────────────┬──────────┬───────────┬───────────────────────────────┐
        │   Archetype    │ midpoint │ curvature │           Behavior            │
        ├────────────────┼──────────┼───────────┼───────────────────────────────┤
        │ market_maker   │ 3-10     │ 0.8-1.2   │ Sharp cutoff at low inventory │
        ├────────────────┼──────────┼───────────┼───────────────────────────────┤
        │ mean_reverter  │ 5-20     │ 0.5-1.5   │ Moderate pullback             │
        ├────────────────┼──────────┼───────────┼───────────────────────────────┤
        │ trend_follower │ 15-50    │ 0.5-1.5   │ Willing to ride positions     │
        ├────────────────┼──────────┼───────────┼───────────────────────────────┤
        │ noise_trader   │ 10-50    │ 0.5-2.0   │ Variable                      │
        └────────────────┴──────────┴───────────┴───────────────────────────────┘

        The desirability sigmoid is:
        1 / (1 + e^(curvature * (|x| - midpoint)))


    """
    name: str
    weight: float  # fraction of total agents (all weights should sum to ~1.0)
    aggression_range: tuple = (0.1, 0.5)
    mean_reversion_range: tuple = (0.0, 0.5)
    trend_follow_range: tuple = (0.0, 0.5)
    noise_scale_range: tuple = (0.05, 0.3)
    ema_alpha_range: tuple = (0.01, 0.2)
    fair_value_lr_range: tuple = (0.001, 0.02)
    position_limit_range: tuple = (10.0, 100.0)
    risk_aversion_range: tuple = (0.01, 0.2)
    curvature_range: tuple = (0.5, 2.0)
    midpoint_range: tuple = (5.0, 50.0)
    mm_half_spread_range: Optional[tuple] = None
    mm_quote_size_range: Optional[tuple] = None
    mm_requote_threshold_range: Optional[tuple] = None

    def to_dict(self) -> dict:
        d = {
            "name": self.name,
            "weight": self.weight,
            "aggression": list(self.aggression_range),
            "mean_reversion": list(self.mean_reversion_range),
            "trend_follow": list(self.trend_follow_range),
            "noise_scale": list(self.noise_scale_range),
            "ema_alpha": list(self.ema_alpha_range),
            "fair_value_lr": list(self.fair_value_lr_range),
            "position_limit": list(self.position_limit_range),
            "risk_aversion": list(self.risk_aversion_range),
            "curvature": list(self.curvature_range),
            "midpoint": list(self.midpoint_range),
        }
        if self.mm_half_spread_range is not None:
            d["mm_half_spread"] = list(self.mm_half_spread_range)
        if self.mm_quote_size_range is not None:
            d["mm_quote_size"] = list(self.mm_quote_size_range)
        if self.mm_requote_threshold_range is not None:
            d["mm_requote_threshold"] = list(self.mm_requote_threshold_range)
        return d


# Canonical archetypes matching main.rs defaults.
MEAN_REVERTER = ArchetypeConfig(
    name="mean_reverter", weight=0.50,
    aggression_range=(0.1, 0.5),
    mean_reversion_range=(0.3, 0.8),
    trend_follow_range=(0.0, 0.0),
    noise_scale_range=(0.05, 0.2),
    ema_alpha_range=(0.01, 0.1),
    fair_value_lr_range=(0.001, 0.01),
    position_limit_range=(10.0, 100.0),
    risk_aversion_range=(0.01, 0.1),
    curvature_range=(0.0, 0.0), # (0.5, 1.5),
    midpoint_range=(5.0, 20.0),
)

TREND_FOLLOWER = ArchetypeConfig(
    name="trend_follower", weight=0.0,
    aggression_range=(0.1, 0.5),
    mean_reversion_range=(0.0, 0.0),
    trend_follow_range=(0.2, 0.7),
    noise_scale_range=(0.05, 0.2),
    ema_alpha_range=(0.01, 0.1),
    fair_value_lr_range=(0.001, 0.01),
    position_limit_range=(10.0, 100.0),
    risk_aversion_range=(0.01, 0.1),
    curvature_range=(0.0, 0.0),
    midpoint_range=(15.0, 50.0),
)

MARKET_MAKER = ArchetypeConfig(
    name="market_maker", weight=0.1,
    aggression_range=(0.02, 0.1),
    mean_reversion_range=(0.1, 0.3),
    trend_follow_range=(0.0, 0.0),
    noise_scale_range=(0.05, 0.2),
    ema_alpha_range=(0.01, 0.1),
    fair_value_lr_range=(0.001, 0.01),
    position_limit_range=(5.0, 20.0),
    risk_aversion_range=(0.05, 0.2),
    curvature_range=(0.0, 0.0),
    midpoint_range=(3.0, 10.0),
    mm_half_spread_range=(0.05, 0.2),
    mm_quote_size_range=(1.0, 5.0),
    mm_requote_threshold_range=(0.05, 0.2),
)

NOISE_TRADER = ArchetypeConfig(
    name="noise_trader", weight=0.4,
    aggression_range=(0.1, 0.5),
    mean_reversion_range=(0.0, 0.0),
    trend_follow_range=(0.0, 0.0),
    noise_scale_range=(15.0, 30.0),
    ema_alpha_range=(0.01, 0.1),
    fair_value_lr_range=(0.001, 0.01),
    position_limit_range=(10.0, 100.0),
    risk_aversion_range=(0.01, 0.1),
    curvature_range=(0.0, 0.0),
    midpoint_range=(10.0, 50.0),
)

DEFAULT_ARCHETYPES = [MEAN_REVERTER, TREND_FOLLOWER, MARKET_MAKER, NOISE_TRADER]


@dataclass
class SimConfig:
    n_agents: int = 100
    initial_price: float = 100.0
    initial_cash: float = 10_000.0
    k: int = 10
    m: int = 4
    use_gpu: bool = True
    seed: Optional[int] = 53
    fair_value_vol: float = 0.002
    init_bias: float = 0.02
    archetypes: Optional[list] = field(default_factory=lambda: list(DEFAULT_ARCHETYPES))
    market_order_threshold: float = 0.0
    participation_threshold: float = 0.5
    tick_size: float = 0.01

    def to_json(self) -> str:
        d = {
            "n_agents": self.n_agents,
            "initial_price": self.initial_price,
            "initial_cash": self.initial_cash,
            "k": self.k,
            "m": self.m,
            "use_gpu": self.use_gpu,
            "seed": self.seed,
            "fair_value_vol": self.fair_value_vol,
            "init_bias": self.init_bias,
            "market_order_threshold": self.market_order_threshold,
            "participation_threshold": self.participation_threshold,
            "tick_size": self.tick_size,
        }
        if self.archetypes is not None:
            d["archetypes"] = [a.to_dict() for a in self.archetypes]
        return json.dumps(d)
