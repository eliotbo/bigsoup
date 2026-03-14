import numpy as np
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
