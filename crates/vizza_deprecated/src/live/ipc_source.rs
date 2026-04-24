//! IPC-based live data source for receiving trades from external processes.

use lod::{LevelStore, PlotTrade};
use std::sync::{Arc, Mutex, mpsc};

use super::{LiveDataSource, Result};

/// A LiveDataSource that receives data from an external process via channels.
///
/// This implementation is designed for inter-process communication where
/// an external process sends PlotTrade data through a channel.
pub struct IPCLiveDataSource {
    receiver: Arc<Mutex<mpsc::Receiver<PlotTrade>>>,
    ticker: String,
    base_price: f64,
    trades_buffer: Vec<PlotTrade>,
}

impl IPCLiveDataSource {
    /// Create a new IPC data source from a channel receiver.
    ///
    /// # Arguments
    /// * `receiver` - Channel receiver for incoming PlotTrade data
    pub fn new(receiver: mpsc::Receiver<PlotTrade>) -> Self {
        Self {
            receiver: Arc::new(Mutex::new(receiver)),
            ticker: String::new(),
            base_price: 100.0, // Default price if no history
            trades_buffer: Vec::new(),
        }
    }

    /// Create a new IPC data source with a specific base price.
    ///
    /// # Arguments
    /// * `receiver` - Channel receiver for incoming PlotTrade data
    /// * `base_price` - Initial reference price
    pub fn with_base_price(receiver: mpsc::Receiver<PlotTrade>, base_price: f64) -> Self {
        Self {
            receiver: Arc::new(Mutex::new(receiver)),
            ticker: String::new(),
            base_price,
            trades_buffer: Vec::new(),
        }
    }
}

impl LiveDataSource for IPCLiveDataSource {
    fn initialize(&mut self, historical_data: &LevelStore, ticker: &str) -> Result<()> {
        self.ticker = ticker.to_string();

        // Try to get base price from historical data if available
        if let Some(candles) = historical_data.get(60) {
            if let Some(last_candle) = candles.last() {
                self.base_price = last_candle.close as f64; // Use last close price
            }
        }

        Ok(())
    }

    fn get_trades(&mut self, _now_ns: i64) -> Vec<PlotTrade> {
        // Drain all available trades from receiver
        if let Ok(rx) = self.receiver.lock() {
            while let Ok(trade) = rx.try_recv() {
                self.trades_buffer.push(trade);
            }
        }
        std::mem::take(&mut self.trades_buffer)
    }

    fn is_market_open(&self, _now_ns: i64) -> bool {
        // For IPC sources, assume market is always open
        // The external process controls when to send data
        true
    }

    fn source_name(&self) -> &str {
        "IPCLiveDataSource"
    }

    fn current_price(&self) -> f64 {
        self.base_price
    }
}
