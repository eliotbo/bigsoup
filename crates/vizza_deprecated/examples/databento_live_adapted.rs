//! Databento Live Data Integration (Adapted for databento-rs Architecture)
//!
//! This example shows how to integrate vizza with the `databento` crate
//! to display live market data in real-time charts.
//!
//! ## Architecture Overview
//!
//! ```
//! Databento Live Gateway
//!     ↓ (databento crate)
//! LiveClient
//!     ↓ (async stream: TradeMsg)
//! DatabentoTradeReceiver
//!     ↓ (converts TradeMsg → PlotTrade)
//! DatabentoDataSource (implements LiveDataSource)
//!     ↓
//! LiveDataManager
//!     ↓
//! Vizza Chart
//! ```
//!
//! ## Setup
//!
//! 1. Add dependencies in vizza/Cargo.toml:
//!    ```toml
//!    [dependencies]
//!    databento = { version = "0.34", features = ["live"] }
//!    tokio = { version = "1", features = ["sync", "rt-multi-thread"] }
//!    ```
//!
//! 2. Set your Databento API key:
//!    ```bash
//!    export DATABENTO_API_KEY="your-key-here"
//!    ```
//!
//! 3. Run:
//!    ```bash
//!    cargo run --example databento_live_adapted
//!    ```

use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

// Vizza imports
use lod::{LevelStore, PlotTrade};
use vizza::live::{LiveDataError, LiveDataSource, Result as LiveResult};
use vizza::{PlotBuilder, live_view::LiveDataManager};

// ============================================================================
// Databento Trade Receiver - Bridges databento to vizza
// ============================================================================

/// Receives TradeMsg from databento LiveClient and converts to PlotTrades
pub struct DatabentoTradeReceiver {
    // Receiver for trades from databento client
    trade_rx: Arc<Mutex<mpsc::UnboundedReceiver<DatabentoTrade>>>,

    // Buffer for converted trades
    trade_buffer: Vec<PlotTrade>,
}

/// Simplified trade structure from databento
#[derive(Debug, Clone)]
pub struct DatabentoTrade {
    pub symbol: String,
    pub price: f64,
    pub size: u32,
    pub timestamp_ns: i64,
    pub flags: u8,
}

impl DatabentoTradeReceiver {
    pub fn new(trade_rx: mpsc::UnboundedReceiver<DatabentoTrade>) -> Self {
        Self {
            trade_rx: Arc::new(Mutex::new(trade_rx)),
            trade_buffer: Vec::new(),
        }
    }

    /// Convert databento trades to vizza PlotTrades
    fn process_trades(&mut self) -> Vec<PlotTrade> {
        let mut new_trades = Vec::new();

        // Drain all available trades from channel
        let trades: Vec<DatabentoTrade> = {
            let mut rx = self.trade_rx.lock().unwrap();
            let mut trades = Vec::new();
            while let Ok(trade) = rx.try_recv() {
                trades.push(trade);
            }
            trades
        };

        for trade in trades {
            let plot_trade = PlotTrade {
                ts: trade.timestamp_ns,
                price: trade.price as f32,
                size: trade.size as f32,
                flags: trade.flags as u32,
            };
            new_trades.push(plot_trade);
        }

        new_trades
    }
}

// ============================================================================
// Databento Data Source - Implements LiveDataSource for vizza
// ============================================================================

pub struct DatabentoDataSource {
    ticker: String,
    base_price: f64,

    // Trade receiver from databento
    trade_receiver: DatabentoTradeReceiver,

    // Connection info (for reconnection if needed)
    dataset: String,
    api_key: String,

    initialized: bool,
}

impl DatabentoDataSource {
    /// Create a new Databento data source
    ///
    /// # Arguments
    /// * `trade_rx` - Receiver for trades from databento LiveClient
    /// * `dataset` - Dataset identifier (e.g., "GLBX.MDP3")
    /// * `api_key` - Databento API key
    pub fn new(
        trade_rx: mpsc::UnboundedReceiver<DatabentoTrade>,
        dataset: &str,
        api_key: &str,
    ) -> Self {
        Self {
            ticker: String::new(),
            base_price: 0.0,
            trade_receiver: DatabentoTradeReceiver::new(trade_rx),
            dataset: dataset.to_string(),
            api_key: api_key.to_string(),
            initialized: false,
        }
    }
}

impl LiveDataSource for DatabentoDataSource {
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
            "✓ DatabentoDataSource initialized: ticker={}, base_price={:.2}",
            self.ticker, self.base_price
        );
        eprintln!(
            "  Dataset: {} (API key: {}...)",
            self.dataset,
            &self.api_key.chars().take(8).collect::<String>()
        );

        Ok(())
    }

    fn get_trades(&mut self, _now_ns: i64) -> Vec<PlotTrade> {
        if !self.initialized {
            return Vec::new();
        }

        // Process trades from databento
        self.trade_receiver.process_trades()
    }

    fn is_market_open(&self, _now_ns: i64) -> bool {
        // For live Databento data, always return true
        // Databento naturally stops sending trades when market closes
        true
    }

    fn source_name(&self) -> &str {
        "DatabentoDataSource"
    }

    fn current_price(&self) -> f64 {
        self.base_price
    }
}

// ============================================================================
// Integration Example
// ============================================================================

/// Example showing full integration with databento
#[tokio::main]
async fn main() -> Result<()> {
    println!("\n=== Databento Live Data Integration ===\n");
    println!("This example shows how to bridge databento-rs and vizza.");
    println!();
    println!("NOTE: This is a demonstration of the integration pattern.");
    println!("In a real application, you would:");
    println!("  1. Start databento LiveClient in a tokio task");
    println!("  2. Subscribe to market data for your ticker");
    println!("  3. Bridge TradeMsg to DatabentoDataSource");
    println!("  4. Run vizza with the Databento data source\n");

    example_integration_pattern().await?;

    Ok(())
}

/// Shows the integration pattern
async fn example_integration_pattern() -> Result<()> {
    println!("Integration Pattern:\n");

    // Step 1: Create channel for trades
    let (trade_tx, trade_rx) = mpsc::unbounded_channel::<DatabentoTrade>();

    println!("1. Created trade channel");

    // Step 2: In a real app, you would spawn databento LiveClient here
    // and listen for TradeMsg, converting them to DatabentoTrade
    //
    // tokio::spawn(async move {
    //     use databento::dbn::{Record, TradeMsg};
    //     use databento::live::LiveClient;
    //     use databento::{Dataset, Schema, Subscription, SType};
    //
    //     let mut client = LiveClient::builder()
    //         .key_from_env()
    //         .expect("DATABENTO_API_KEY not set")
    //         .dataset(Dataset::GlbxMdp3)
    //         .build()
    //         .await
    //         .expect("Failed to build LiveClient");
    //
    //     client
    //         .subscribe(
    //             Subscription::builder()
    //                 .symbols("ES.FUT")
    //                 .schema(Schema::Trades)
    //                 .stype_in(SType::Parent)
    //                 .build(),
    //         )
    //         .await
    //         .expect("Failed to subscribe");
    //
    //     client.start().await.expect("Failed to start client");
    //
    //     // Get symbol map for resolving instrument IDs
    //     let symbol_map = client.symbol_map();
    //
    //     while let Some(record) = client.next_record().await.transpose() {
    //         match record {
    //             Ok(rec) => {
    //                 if let Some(trade) = rec.get::<TradeMsg>() {
    //                     // Resolve symbol from instrument ID
    //                     let symbol = symbol_map
    //                         .get(trade.hd.instrument_id)
    //                         .map(|s| s.to_string())
    //                         .unwrap_or_else(|| format!("{}", trade.hd.instrument_id));
    //
    //                     let databento_trade = DatabentoTrade {
    //                         symbol,
    //                         price: trade.price as f64 / 1_000_000_000.0, // Convert from fixed-point
    //                         size: trade.size,
    //                         timestamp_ns: trade.ts_event,
    //                         flags: trade.flags,
    //                     };
    //
    //                     trade_tx.send(databento_trade).ok();
    //                 }
    //             }
    //             Err(e) => {
    //                 eprintln!("Error receiving record: {}", e);
    //                 break;
    //             }
    //         }
    //     }
    // });

    println!("2. (Would spawn databento LiveClient here)");

    // Step 3: Create Databento data source with trade receiver
    let api_key =
        std::env::var("DATABENTO_API_KEY").unwrap_or_else(|_| "db-YOUR-KEY-HERE".to_string());

    let databento_source = Box::new(DatabentoDataSource::new(trade_rx, "GLBX.MDP3", &api_key));

    println!("3. Created DatabentoDataSource");

    // Step 4: Use with vizza
    println!("4. Ready to use with vizza:");
    println!();
    println!("   let market_data = vizza::loader::load_market_data()?;");
    println!("   let live_manager = LiveDataManager::with_data_source(");
    println!("       Arc::clone(&market_data.level_store),");
    println!("       databento_source,");
    println!("       \"ES.FUT\",");
    println!("   );");
    println!();
    println!("   PlotBuilder::new()");
    println!("       .with_data_paths(vec![\"historical_data.nohlcv\".to_string()])");
    println!("       .with_live_data(true, Some(100))");
    println!("       .run()?;");

    Ok(())
}

// ============================================================================
// Complete Working Example (Requires databento API key)
// ============================================================================

/// Complete example that could work with real databento connection
#[allow(dead_code)]
async fn example_with_real_connection() -> Result<()> {
    use tokio::sync::mpsc;

    // Create trade bridge channel
    let (trade_tx, trade_rx) = mpsc::unbounded_channel::<DatabentoTrade>();

    // Spawn databento LiveClient task
    // NOTE: This requires the databento crate with "live" feature
    //
    // tokio::spawn(async move {
    //     use databento::dbn::{Record, TradeMsg};
    //     use databento::live::LiveClient;
    //     use databento::{Dataset, Schema, Subscription, SType};
    //
    //     // Build client
    //     let mut client = match LiveClient::builder()
    //         .key_from_env()
    //         .and_then(|b| Ok(b.dataset(Dataset::GlbxMdp3)))
    //         .and_then(|b| b.build())
    //         .await
    //     {
    //         Ok(c) => c,
    //         Err(e) => {
    //             eprintln!("Failed to build LiveClient: {}", e);
    //             return;
    //         }
    //     };
    //
    //     // Subscribe to trades
    //     if let Err(e) = client
    //         .subscribe(
    //             Subscription::builder()
    //                 .symbols("ES.FUT")
    //                 .schema(Schema::Trades)
    //                 .stype_in(SType::Parent)
    //                 .build(),
    //         )
    //         .await
    //     {
    //         eprintln!("Failed to subscribe: {}", e);
    //         return;
    //     }
    //
    //     // Start streaming
    //     if let Err(e) = client.start().await {
    //         eprintln!("Failed to start client: {}", e);
    //         return;
    //     }
    //
    //     let symbol_map = client.symbol_map();
    //
    //     // Process trades
    //     while let Some(record_result) = client.next_record().await.transpose() {
    //         match record_result {
    //             Ok(rec) => {
    //                 if let Some(trade) = rec.get::<TradeMsg>() {
    //                     let symbol = symbol_map
    //                         .get(trade.hd.instrument_id)
    //                         .map(|s| s.to_string())
    //                         .unwrap_or_else(|| format!("{}", trade.hd.instrument_id));
    //
    //                     let databento_trade = DatabentoTrade {
    //                         symbol,
    //                         price: trade.price as f64 / 1_000_000_000.0,
    //                         size: trade.size,
    //                         timestamp_ns: trade.ts_event,
    //                         flags: trade.flags,
    //                     };
    //
    //                     let _ = trade_tx.send(databento_trade);
    //                 }
    //             }
    //             Err(e) => {
    //                 eprintln!("Error receiving record: {}", e);
    //                 break;
    //             }
    //         }
    //     }
    // });

    // Create Databento data source
    let api_key = std::env::var("DATABENTO_API_KEY")?;
    let databento_source = Box::new(DatabentoDataSource::new(trade_rx, "GLBX.MDP3", &api_key));

    // Use with vizza (would need to be in a blocking context)
    // PlotBuilder::new()
    //     .with_data_paths(vec!["historical_data.nohlcv".to_string()])
    //     .with_live_data(true, Some(100))
    //     .run()?;

    Ok(())
}

// ============================================================================
// Helper: Bridge TradeMsg to DatabentoTrade
// ============================================================================

/// Helper function to bridge databento TradeMsg to vizza
///
/// Call this in your application where you spawn the databento LiveClient:
///
/// ```rust,ignore
/// use databento::dbn::{Record, TradeMsg};
///
/// let (trade_tx, trade_rx) = mpsc::unbounded_channel();
///
/// tokio::spawn(async move {
///     while let Some(record) = client.next_record().await.transpose() {
///         if let Ok(rec) = record {
///             bridge_databento_trade(rec, &trade_tx, &symbol_map);
///         }
///     }
/// });
/// ```
#[allow(dead_code)]
fn bridge_databento_trade(
    _rec: (), // In real code: RecordRef
    _trade_tx: &mpsc::UnboundedSender<DatabentoTrade>,
    _symbol_map: (), // In real code: &PitSymbolMap
) {
    // In real implementation with databento available:
    //
    // use databento::dbn::TradeMsg;
    //
    // if let Some(trade) = rec.get::<TradeMsg>() {
    //     let symbol = symbol_map
    //         .get(trade.hd.instrument_id)
    //         .map(|s| s.to_string())
    //         .unwrap_or_else(|| format!("{}", trade.hd.instrument_id));
    //
    //     let databento_trade = DatabentoTrade {
    //         symbol,
    //         price: trade.price as f64 / 1_000_000_000.0, // Convert from fixed-point
    //         size: trade.size,
    //         timestamp_ns: trade.ts_event,
    //         flags: trade.flags,
    //     };
    //
    //     // Send to vizza data source
    //     let _ = trade_tx.send(databento_trade);
    // }
}
