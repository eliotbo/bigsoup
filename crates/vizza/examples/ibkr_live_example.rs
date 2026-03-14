//! IBKR Live Data Example
//!
//! This example shows how to integrate real Interactive Brokers live data
//! into vizza using a custom LiveDataSource implementation.
//!
//! ## Prerequisites
//!
//! 1. **Install IBKR TWS or IB Gateway**
//!    - Download from: https://www.interactivebrokers.com/en/trading/tws.php
//!    - Use paper trading account for testing
//!
//! 2. **Enable API Access**
//!    - TWS: File → Global Configuration → API → Settings
//!    - Enable "Enable ActiveX and Socket Clients"
//!    - Set Socket port: 7497 (paper) or 7496 (live)
//!    - Add 127.0.0.1 to trusted IPs
//!
//! 3. **Add Dependencies** (add to your Cargo.toml)
//!    ```toml
//!    [dependencies]
//!    ibapi = "0.3"  # Or another IBKR API crate
//!    crossbeam-channel = "0.5"
//!    ```
//!
//! ## Usage
//!
//! ```bash
//! # Start TWS or IB Gateway first, then:
//! cargo run --example ibkr_live_example
//! ```

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::sync::{Arc, Mutex};

// Import vizza components
use lod::{LevelStore, PlotTrade};
use vizza::live::{LiveDataError, LiveDataSource, Result as LiveResult};
use vizza::{PlotBuilder, live_view::LiveDataManager};

// ============================================================================
// IBKR Data Source Implementation
// ============================================================================

/// Live data source that connects to Interactive Brokers TWS/Gateway
pub struct IBKRDataSource {
    ticker: String,
    base_price: f64,

    // Channel for thread-safe trade buffering
    trade_rx: Receiver<PlotTrade>,
    trade_tx: Sender<PlotTrade>,

    // Connection config
    host: String,
    port: u16,
    client_id: i32,

    initialized: bool,
}

impl IBKRDataSource {
    /// Create a new IBKR data source
    ///
    /// # Arguments
    /// * `host` - TWS/Gateway host (usually "127.0.0.1")
    /// * `port` - API port (7497 for TWS paper, 4002 for Gateway paper)
    /// * `client_id` - Unique client ID (any positive integer)
    pub fn new(host: &str, port: u16, client_id: i32) -> Self {
        let (tx, rx) = unbounded();

        Self {
            ticker: String::new(),
            base_price: 0.0,
            trade_rx: rx,
            trade_tx: tx,
            host: host.to_string(),
            port,
            client_id,
            initialized: false,
        }
    }

    /// Connect to IBKR and subscribe to market data
    fn connect_and_subscribe(&mut self) -> LiveResult<()> {
        eprintln!("📡 Connecting to IBKR at {}:{}...", self.host, self.port);

        // NOTE: This is a placeholder. In a real implementation:
        //
        // 1. Use ibapi crate:
        //    ```
        //    use ibapi::Client;
        //    let mut client = Client::connect(&self.host, self.port, self.client_id)?;
        //    ```
        //
        // 2. Create contract:
        //    ```
        //    let contract = Contract::stock(&self.ticker);
        //    ```
        //
        // 3. Subscribe to tick-by-tick:
        //    ```
        //    let tx = self.trade_tx.clone();
        //    client.req_tick_by_tick_data(1, &contract, "AllLast", 0, false,
        //        move |tick| {
        //            let trade = PlotTrade {
        //                ts: tick.time * 1_000_000_000,
        //                price: tick.price as f32,
        //                size: tick.size as f32,
        //                flags: 0,
        //            };
        //            tx.send(trade).ok();
        //        }
        //    )?;
        //    ```

        eprintln!("✓ Connected to IBKR (mock - implement with ibapi crate)");
        eprintln!("✓ Subscribed to tick-by-tick data for {}", self.ticker);

        Ok(())
    }
}

impl LiveDataSource for IBKRDataSource {
    fn initialize(&mut self, historical_data: &LevelStore, ticker: &str) -> LiveResult<()> {
        self.ticker = ticker.to_string();

        // Get base price from historical data
        if let Some(candles) = historical_data.get(60) {
            if let Some(last) = candles.last() {
                self.base_price = last.close as f64;
            }
        }

        // Connect to IBKR and subscribe to market data
        self.connect_and_subscribe()?;

        self.initialized = true;

        eprintln!(
            "✓ IBKRDataSource initialized: ticker={}, base_price={:.2}",
            self.ticker, self.base_price
        );

        Ok(())
    }

    fn get_trades(&mut self, _now_ns: i64) -> Vec<PlotTrade> {
        if !self.initialized {
            return Vec::new();
        }

        // Drain all available trades from channel
        // This is thread-safe - IBKR callbacks run on different thread
        let trades: Vec<PlotTrade> = self.trade_rx.try_iter().collect();

        if !trades.is_empty() {
            static mut FIRST_BATCH_LOGGED: bool = false;
            unsafe {
                if !FIRST_BATCH_LOGGED {
                    eprintln!("✓ Received {} trades from IBKR", trades.len());
                    FIRST_BATCH_LOGGED = true;
                }
            }
        }

        trades
    }

    fn is_market_open(&self, _now_ns: i64) -> bool {
        // For live IBKR data, always return true
        // IBKR will naturally stop sending ticks when market closes
        true
    }

    fn source_name(&self) -> &str {
        "IBKRDataSource"
    }

    fn current_price(&self) -> f64 {
        self.base_price
    }
}

// ============================================================================
// Example Usage
// ============================================================================

fn main() -> Result<()> {
    println!("\n=== IBKR Live Data Example ===\n");
    println!("NOTE: This is a demonstration. To use real IBKR data:");
    println!("  1. Add 'ibapi' crate to Cargo.toml");
    println!("  2. Implement actual IBKR connection in IBKRDataSource");
    println!("  3. Start TWS or IB Gateway before running\n");

    // Example 1: Paper trading connection
    example_paper_trading()?;

    Ok(())
}

/// Example: Connect to paper trading account
fn example_paper_trading() -> Result<()> {
    println!("Example 1: IBKR Paper Trading\n");

    let base = "../../data/consolidated/stock-split-dividend-test/";
    let data_path = format!("{}AAPL/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base);

    // Create IBKR data source
    // Port 7497 = TWS paper trading
    // Port 4002 = IB Gateway paper trading
    let ibkr_source = Box::new(IBKRDataSource::new(
        "127.0.0.1", // Localhost
        7497,        // TWS paper trading port
        1,           // Client ID
    ));

    // Note: In a real implementation, you would:
    //
    // 1. Load historical data
    // 2. Create LiveDataManager with IBKR source
    // 3. Pass to renderer
    //
    // Like this:
    //
    // let market_data = vizza::loader::load_market_data()?;
    // let live_manager = LiveDataManager::with_data_source(
    //     Arc::clone(&market_data.level_store),
    //     ibkr_source,
    //     "AAPL",
    // );

    // For now, just demonstrate the interface
    PlotBuilder::new()
        .with_data_paths(vec![data_path])
        .with_live_data(true, Some(100)) // 100ms update interval
        .run()?;

    Ok(())
}

/// Example: Multiple tickers (future enhancement)
#[allow(dead_code)]
fn example_multi_ticker() {
    println!("Example 2: Multiple Tickers (Future)\n");
    println!("To support multiple tickers:");
    println!("  1. Create IBKRDataSource for each ticker");
    println!("  2. Assign to different viewports");
    println!("  3. Use different client_ids to avoid conflicts\n");

    // Pseudocode:
    // let aapl_source = IBKRDataSource::new("127.0.0.1", 7497, 1);
    // let msft_source = IBKRDataSource::new("127.0.0.1", 7497, 2);
    // let googl_source = IBKRDataSource::new("127.0.0.1", 7497, 3);
    //
    // // Assign to viewports...
}

// ============================================================================
// Real Implementation Checklist
// ============================================================================
//
// To implement real IBKR integration:
//
// [ ] 1. Add dependency to Cargo.toml:
//        ibapi = "0.3"
//
// [ ] 2. Implement connect_and_subscribe():
//        - Create ibapi::Client
//        - Connect to TWS/Gateway
//        - Create Contract for ticker
//        - Subscribe to tick-by-tick data
//        - Handle callbacks in separate thread
//
// [ ] 3. Handle errors:
//        - Connection failures
//        - Market data permissions
//        - Invalid ticker symbols
//        - Disconnections
//
// [ ] 4. Add reconnection logic:
//        - Detect when connection drops
//        - Automatically reconnect
//        - Resubscribe to data
//
// [ ] 5. Test with paper trading account first
//
// [ ] 6. Consider data rate limits:
//        - IBKR has rate limits for market data
//        - May need to throttle or aggregate ticks
//
// ============================================================================
