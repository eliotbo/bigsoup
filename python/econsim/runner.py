"""
econsim Python frontend
=======================

Build (run once, and again after any Rust change):

    cd /workspace/workspace/bigsoup
    maturin develop --release --skip-install

The .so lands in python/econsim/ automatically.  Then run:

    PYTHONPATH=python python3 python/econsim/runner.py

Quick start (in a script):

    from econsim.runner import run_simulation
    results = run_simulation(n_ticks=1000)
    print(results["prices"][-1])

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

import numpy as np

if __name__ == "__main__":
    from config import SimConfig
    from econsim import PySimulation
else:
    from .config import SimConfig
    from .econsim import PySimulation


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
    r = run_simulation(n_ticks=10000)
    print(f'final price:  {r["final_price"]:.4f}')
    print(f'price std:    {r["price_std"]:.4f}')
    print(f'total volume: {r["total_volume"]:.0f}')
    print(f'bbo:          {r["bbo"]}')
