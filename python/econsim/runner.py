"""
econsim Python frontend
=======================

Build (run once, and again after any Rust change):

    cd /workspace/workspace/bigsoup
    maturin develop --release --skip-install

The .so lands in python/econsim/ automatically.  Then run:

    PYTHONPATH=python python3 python/econsim/runner.py

Custom archetypes:

    from econsim import SimConfig, MEAN_REVERTER, NOISE_TRADER
    from econsim.runner import run_simulation

    config = SimConfig(
        n_agents=50_000,
        archetypes=[MEAN_REVERTER, NOISE_TRADER],
    )
    results = run_simulation(config, n_ticks=500)

Return value of run_simulation:
    {
        "prices":      np.ndarray[f32],  # last_price per tick
        "volumes":     np.ndarray[f32],  # total matched volume per tick
        "final_price": float,
        "price_std":   float,
        "total_volume":float,
        "n_ticks":     int,
        "bbo":         (best_bid, best_bid_size, best_ask, best_ask_size, last_price),
    }
"""

import os
import subprocess
import numpy as np

if __name__ == "__main__":
    from config import SimConfig
    from econsim import PySimulation
else:
    from .config import SimConfig
    from .econsim import PySimulation

_REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def show_chart(config: SimConfig = None, n_ticks: int = 10_000, strategy: str = None):
    """Launch the vizza candlestick chart for a simulation run."""
    if config is None:
        config = SimConfig()
    args = [
        "cargo", "run", "--bin", "chart", "--",
        "--ticks", str(n_ticks),
        "--config", config.to_json(),
    ]
    if strategy is not None:
        args += ["--strategy", strategy]
    subprocess.run(args, cwd=_REPO_ROOT)


def run_simulation(config: SimConfig = None, n_ticks: int = 1000) -> dict:
    if config is None:
        config = SimConfig()

    sim = PySimulation(config.to_json())
    sim.run(n_ticks)

    prices = np.asarray(sim.price_history())
    volumes = np.asarray(sim.volume_history())

    return {
        "prices": prices,
        "volumes": volumes,
        "final_price": float(prices[-1]) if len(prices) > 0 else None,
        "price_std": float(prices.std()) if len(prices) > 0 else None,
        "total_volume": float(volumes.sum()),
        "n_ticks": sim.tick(),
        "bbo": sim.bbo(),
    }


if __name__ == "__main__":
    from dsl import Param, State, BboField, Noise, Position, Abs, Exp, signal, compile

    mr = Param("mean_reversion") * (State("fair_value_estimate") - BboField("mid"))
    tf = Param("trend_follow") * (BboField("mid") - State("ema"))
    raw = mr + tf + Noise() + (-Param("risk_aversion") * Position())
    desirability = 1.0 / (1.0 + Exp(Param("curvature") * (Abs(Position()) - Param("midpoint"))))
    c_str = compile(signal(raw * desirability))

    print(f'DSL signal: {c_str}')

    config = SimConfig()
    show_chart(config, n_ticks=100_000, strategy=c_str)
