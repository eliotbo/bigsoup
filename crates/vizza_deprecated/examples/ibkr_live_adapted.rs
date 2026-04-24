//! IBKR Live Data Integration (Adapted for ibkr_proto Architecture)
//!
//! This example shows how to integrate vizza with the existing `ibkr_proto` crate
//! to display live IBKR data in real-time charts.
//!
//! ## Architecture Overview
//!
//! ```
//! IBKR Gateway/TWS
//!     ↓ (ibapi crate)
//! ibkr_proto::Backend
//!     ↓ (mpsc channel: BackendMessage::Quote)
//! IBKRQuoteReceiver
//!     ↓ (converts LiveQuote → PlotTrade)
//! IBKRDataSource (implements LiveDataSource)
//!     ↓
//! LiveDataManager
//!     ↓
//! Vizza Chart
//! ```
//!
//! ## Setup
//!
//! 1. Add dependency in vizza/Cargo.toml:
//!    ```toml
//!    [dependencies]
//!    ibkr_proto = { path = "../ibkr_proto" }
//!    tokio = { version = "1", features = ["sync"] }
//!    ```
//!
//! 2. Start IB Gateway or TWS
//!
//! 3. Run:
//!    ```bash
//!    cargo run --example ibkr_live_adapted
//!    ```

use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

// Vizza imports
use lod::{LevelStore, PlotTrade};
use vizza::live::{LiveDataError, LiveDataSource, Result as LiveResult};
use vizza::{PlotBuilder, live_view::LiveDataManager};

// ============================================================================
// IBKR Quote Receiver - Bridges ibkr_proto to vizza
// ============================================================================

/// Receives BackendMessage::Quote from ibkr_proto and converts to PlotTrades
pub struct IBKRQuoteReceiver {
    // Receiver for quotes from ibkr_proto backend
    quote_rx: Arc<Mutex<mpsc::UnboundedReceiver<IBKRQuote>>>,

    // Buffer for converted trades
    trade_buffer: Vec<PlotTrade>,

    // Track last quote for conversion
    last_quote: Option<IBKRQuote>,
}

/// Simplified quote structure from ibkr_proto
#[derive(Debug, Clone)]
pub struct IBKRQuote {
    pub symbol: String,
    pub last: Option<f64>,
    pub volume: Option<i64>,
    pub timestamp_ns: i64,
}

impl IBKRQuoteReceiver {
    pub fn new(quote_rx: mpsc::UnboundedReceiver<IBKRQuote>) -> Self {
        Self {
            quote_rx: Arc::new(Mutex::new(quote_rx)),
            trade_buffer: Vec::new(),
            last_quote: None,
        }
    }

    /// Convert quotes to trades
    ///
    /// IBKR sends periodic quote snapshots (bid/ask/last/volume).
    /// We convert these to synthetic "trade" events for vizza.
    fn process_quotes(&mut self) -> Vec<PlotTrade> {
        let mut new_trades = Vec::new();

        // Drain all available quotes from channel
        let quotes: Vec<IBKRQuote> = {
            let mut rx = self.quote_rx.lock().unwrap();
            let mut quotes = Vec::new();
            while let Ok(quote) = rx.try_recv() {
                quotes.push(quote);
            }
            quotes
        };

        for quote in quotes {
            // Convert quote to trade if we have a last price
            if let Some(last_price) = quote.last {
                // Calculate volume delta from previous quote
                let size = if let Some(prev) = &self.last_quote {
                    quote.volume.unwrap_or(0) - prev.volume.unwrap_or(0)
                } else {
                    quote.volume.unwrap_or(0)
                };

                // Only create trade if we have meaningful data
                if size > 0 {
                    let trade = PlotTrade {
                        ts: quote.timestamp_ns,
                        price: last_price as f32,
                        size: size as f32,
                        flags: 0,
                    };
                    new_trades.push(trade);
                }
            }

            self.last_quote = Some(quote);
        }

        new_trades
    }
}

// ============================================================================
// IBKR Data Source - Implements LiveDataSource for vizza
// ============================================================================

pub struct IBKRDataSource {
    ticker: String,
    base_price: f64,

    // Quote receiver from ibkr_proto
    quote_receiver: IBKRQuoteReceiver,

    // Connection info (for reconnection if needed)
    host: String,
    port: u16,
    client_id: i32,

    initialized: bool,
}

impl IBKRDataSource {
    /// Create a new IBKR data source
    ///
    /// # Arguments
    /// * `quote_rx` - Receiver for quotes from ibkr_proto backend
    /// * `host` - IBKR Gateway/TWS host
    /// * `port` - API port
    /// * `client_id` - Client ID
    pub fn new(
        quote_rx: mpsc::UnboundedReceiver<IBKRQuote>,
        host: &str,
        port: u16,
        client_id: i32,
    ) -> Self {
        Self {
            ticker: String::new(),
            base_price: 0.0,
            quote_receiver: IBKRQuoteReceiver::new(quote_rx),
            host: host.to_string(),
            port,
            client_id,
            initialized: false,
        }
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

        self.initialized = true;

        eprintln!(
            "✓ IBKRDataSource initialized: ticker={}, base_price={:.2}",
            self.ticker, self.base_price
        );
        eprintln!(
            "  Connection: {}:{} (client_id={})",
            self.host, self.port, self.client_id
        );

        Ok(())
    }

    fn get_trades(&mut self, _now_ns: i64) -> Vec<PlotTrade> {
        if !self.initialized {
            return Vec::new();
        }

        // Process quotes and convert to trades
        self.quote_receiver.process_quotes()
    }

    fn is_market_open(&self, _now_ns: i64) -> bool {
        // For live IBKR data, always return true
        // IBKR naturally stops sending quotes when market closes
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
// Integration Example
// ============================================================================

/// Example showing full integration with ibkr_proto
#[tokio::main]
async fn main() -> Result<()> {
    println!("\n=== IBKR Live Data Integration (ibkr_proto) ===\n");
    println!("This example shows how to bridge ibkr_proto and vizza.");
    println!();
    println!("NOTE: This is a demonstration of the integration pattern.");
    println!("In a real application, you would:");
    println!("  1. Start ibkr_proto Backend in a tokio task");
    println!("  2. Subscribe to market data for your ticker");
    println!("  3. Bridge BackendMessage::Quote to IBKRDataSource");
    println!("  4. Run vizza with the IBKR data source\n");

    example_integration_pattern().await?;

    Ok(())
}

/// Shows the integration pattern
async fn example_integration_pattern() -> Result<()> {
    println!("Integration Pattern:\n");

    // Step 1: Create channel for quotes
    let (quote_tx, quote_rx) = mpsc::unbounded_channel::<IBKRQuote>();

    println!("1. Created quote channel");

    // Step 2: In a real app, you would spawn ibkr_proto Backend here
    // and listen for BackendMessage::Quote, converting them to IBKRQuote
    //
    // tokio::spawn(async move {
    //     let (ui_tx, mut backend_rx) = mpsc::channel(100);
    //     let backend = ibkr_proto::Backend::new(...);
    //
    //     tokio::spawn(async move {
    //         while let Some(msg) = backend_rx.recv().await {
    //             if let BackendMessage::Quote(live_quote) = msg {
    //                 let ibkr_quote = IBKRQuote {
    //                     symbol: live_quote.symbol,
    //                     last: live_quote.last,
    //                     volume: live_quote.volume,
    //                     timestamp_ns: live_quote.updated_at.unix_timestamp_nanos(),
    //                 };
    //                 quote_tx.send(ibkr_quote).ok();
    //             }
    //         }
    //     });
    //
    //     backend.run().await;
    // });

    println!("2. (Would spawn ibkr_proto Backend here)");

    // Step 3: Create IBKR data source with quote receiver
    let ibkr_source = Box::new(IBKRDataSource::new(
        quote_rx,
        "127.0.0.1",
        7497, // Paper trading port
        100,  // Market data client ID (separate from trading client)
    ));

    println!("3. Created IBKRDataSource");

    // Step 4: Use with vizza
    println!("4. Ready to use with vizza:");
    println!();
    println!("   let market_data = vizza::loader::load_market_data()?;");
    println!("   let live_manager = LiveDataManager::with_data_source(");
    println!("       Arc::clone(&market_data.level_store),");
    println!("       ibkr_source,");
    println!("       \"AAPL\",");
    println!("   );");
    println!();
    println!("   PlotBuilder::new()");
    println!("       .with_data_paths(vec![\"historical_data.nohlcv\".to_string()])");
    println!("       .with_live_data(true, Some(100))");
    println!("       .run()?;");

    Ok(())
}

// ============================================================================
// Complete Working Example (Requires ibkr_proto running)
// ============================================================================

/// Complete example that could work with real ibkr_proto connection
#[allow(dead_code)]
async fn example_with_real_connection() -> Result<()> {
    use tokio::sync::mpsc;

    // Create channels for ibkr_proto communication
    let (ui_tx, ui_rx) = mpsc::channel(100);
    let (backend_tx, mut backend_rx) = mpsc::channel(100);

    // Create quote bridge channel
    let (quote_tx, quote_rx) = mpsc::unbounded_channel::<IBKRQuote>();

    // Spawn ibkr_proto backend
    // NOTE: This requires ibkr_proto to be available as a dependency
    //
    // tokio::spawn(async move {
    //     let benchmark_tracker = ibkr_proto::BenchmarkTracker::new();
    //     let backend = ibkr_proto::Backend::new(ui_rx, backend_tx, benchmark_tracker);
    //     backend.run().await;
    // });

    // Spawn quote bridge - converts BackendMessage to IBKRQuote
    tokio::spawn(async move {
        while let Some(_msg) = backend_rx.recv().await {
            // In real implementation:
            // if let BackendMessage::Quote(live_quote) = msg {
            //     let ibkr_quote = IBKRQuote {
            //         symbol: live_quote.symbol,
            //         last: live_quote.last,
            //         volume: live_quote.volume,
            //         timestamp_ns: live_quote.updated_at.unix_timestamp_nanos(),
            //     };
            //     quote_tx.send(ibkr_quote).ok();
            // }
        }
    });

    // Connect to IBKR
    // ui_tx.send(UiMessage::Connect {
    //     host: "127.0.0.1".to_string(),
    //     port: 7497,
    //     client_id: 1,
    //     market_data_type: ibapi::market_data::MarketDataType::Realtime,
    // }).await?;

    // Subscribe to market data
    // let contract = Contract::stock("AAPL");
    // ui_tx.send(UiMessage::SetContract(contract)).await?;

    // Create IBKR data source
    let ibkr_source = Box::new(IBKRDataSource::new(quote_rx, "127.0.0.1", 7497, 1));

    // Use with vizza (would need to be in a blocking context)
    // PlotBuilder::new()
    //     .with_data_paths(vec!["historical_data.nohlcv".to_string()])
    //     .with_live_data(true, Some(100))
    //     .run()?;

    Ok(())
}

// ============================================================================
// Helper: Bridge BackendMessage to IBKRQuote
// ============================================================================

/// Helper function to bridge ibkr_proto Backend messages to vizza
///
/// Call this in your application where you spawn the ibkr_proto Backend:
///
/// ```rust,ignore
/// let (quote_tx, quote_rx) = mpsc::unbounded_channel();
///
/// tokio::spawn(async move {
///     while let Some(msg) = backend_rx.recv().await {
///         bridge_ibkr_proto_message(msg, &quote_tx);
///     }
/// });
/// ```
#[allow(dead_code)]
fn bridge_ibkr_proto_message(
    _msg: (), // In real code: BackendMessage
    _quote_tx: &mpsc::UnboundedSender<IBKRQuote>,
) {
    // In real implementation with ibkr_proto available:
    //
    // if let BackendMessage::Quote(live_quote) = msg {
    //     let ibkr_quote = IBKRQuote {
    //         symbol: live_quote.symbol,
    //         last: live_quote.last,
    //         volume: live_quote.volume,
    //         timestamp_ns: live_quote.updated_at.unix_timestamp_nanos() as i64,
    //     };
    //
    //     // Send to vizza data source
    //     let _ = quote_tx.send(ibkr_quote);
    // }
}
