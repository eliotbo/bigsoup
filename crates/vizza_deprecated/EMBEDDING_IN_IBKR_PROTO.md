# Embedding Vizza Charts in ibkr_proto
**Status: PROPOSED IMPLEMENTATION - Not Yet Built**

## Overview

This document describes a **proposed architecture** for adding live vizza charts to the existing `ibkr_proto` trading application. Instead of vizza consuming IBKR data, this design would embed vizza as a visualization component within ibkr_proto.

> **⚠️ IMPORTANT**: This integration has NOT been implemented yet. This document serves as a design guide for future implementation.

## Current State (As of October 2025)

### What Exists:
- ✅ `ibkr_proto` has working IBKR connection and market data streaming
- ✅ `vizza` has live data visualization capabilities via `LiveDataSource` trait
- ✅ Example file `examples/ibkr_live_adapted.rs` shows conceptual integration pattern
- ✅ Both crates can run independently

### What Doesn't Exist Yet:
- ❌ No `ChartManager` in ibkr_proto
- ❌ No chart control messages (`OpenChart`, `CloseChart`) in `UiMessage` enum
- ❌ No `charts/` module in ibkr_proto
- ❌ No actual data bridge between ibkr_proto and vizza
- ❌ Dependencies not configured between the crates

## Proposed Implementation Guide

The following sections describe how to implement this integration:

## Proposed Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  ibkr_proto Application                                     │
│                                                              │
│  ┌────────────────┐                                         │
│  │  Backend       │ ← IBKR Gateway/TWS                      │
│  │  (tokio async) │                                         │
│  └───────┬────────┘                                         │
│          │ BackendMessage::Quote                            │
│          ├──────────────┬────────────────┐                  │
│          ↓              ↓                ↓                   │
│  ┌──────────────┐  ┌─────────┐   ┌─────────────────┐       │
│  │  Main UI     │  │ Bridge  │   │ Chart Manager   │       │
│  │  (existing)  │  │         │   │ (new)           │       │
│  └──────────────┘  └────┬────┘   └────────┬────────┘       │
│                          │                 │                │
│                          ↓                 ↓                │
│                    ┌──────────────────────────────┐         │
│                    │  Vizza Chart Windows         │         │
│                    │  (separate threads)          │         │
│                    │  - AAPL chart                │         │
│                    │  - MSFT chart                │         │
│                    │  - TSLA chart                │         │
│                    └──────────────────────────────┘         │
└─────────────────────────────────────────────────────────────┘
```

## Key Design Decisions (Proposed)

### 1. Separate Threads for Charts

Each vizza chart runs in its own thread because:
- Vizza uses winit's blocking event loop
- ibkr_proto uses tokio async runtime
- Multiple charts can run independently

```rust
// Spawn chart in separate thread
let chart_handle = std::thread::spawn(move || {
    run_vizza_chart(ticker, trade_rx)
});
```

### 2. Data Flow via Channels

Use `crossbeam_channel` or `tokio::sync::mpsc` to bridge async backend to sync charts:

```rust
// Create channel for each chart
let (trade_tx, trade_rx) = crossbeam_channel::unbounded();

// Backend sends trades
tokio::spawn(async move {
    while let Some(quote) = quote_stream.next().await {
        let trade = convert_quote_to_trade(quote);
        trade_tx.send(trade).ok();
    }
});

// Vizza receives in separate thread
std::thread::spawn(move || {
    let ibkr_source = IBKRDataSource::new(trade_rx);
    PlotBuilder::new()
        .with_live_data(true, Some(100))
        .run()
});
```

### 3. Chart Manager

Centralized component to manage multiple chart windows:

```rust
pub struct ChartManager {
    charts: HashMap<String, ChartHandle>,
}

impl ChartManager {
    pub fn open_chart(&mut self, ticker: &str, trade_rx: Receiver<PlotTrade>) {
        let handle = std::thread::spawn(move || {
            // Run vizza chart
        });
        self.charts.insert(ticker.to_string(), handle);
    }

    pub fn close_chart(&mut self, ticker: &str) {
        if let Some(handle) = self.charts.remove(ticker) {
            // Signal chart to close
        }
    }
}
```

## Implementation Steps

> **Note**: These are the steps needed to implement the proposed integration. None of these changes have been made yet.

### Step 1: Add vizza Dependency

**TO DO**: Add to `ibkr_proto/Cargo.toml`:

```toml
[dependencies]
vizza = { path = "../vizza" }
lod = { path = "../lod" }
crossbeam-channel = "0.5"
```

### Step 2: Create Chart Bridge Module

**TO DO**: Create new file `ibkr_proto/src/charts/mod.rs`:

```rust
use crossbeam_channel::{Receiver, Sender, unbounded};
use lod::PlotTrade;
use std::collections::HashMap;
use std::thread::JoinHandle;
use vizza::live::{LiveDataError, LiveDataSource, Result as LiveResult};
use vizza::{PlotBuilder, live_view::LiveDataManager};
use lod::LevelStore;

/// Manages vizza chart windows
pub struct ChartManager {
    charts: HashMap<String, ChartInstance>,
}

struct ChartInstance {
    trade_tx: Sender<PlotTrade>,
    handle: JoinHandle<()>,
}

impl ChartManager {
    pub fn new() -> Self {
        Self {
            charts: HashMap::new(),
        }
    }

    /// Open a new chart window for the given ticker
    pub fn open_chart(&mut self, ticker: String, historical_data_path: String) {
        if self.charts.contains_key(&ticker) {
            eprintln!("Chart for {} already open", ticker);
            return;
        }

        let (trade_tx, trade_rx) = unbounded();
        let ticker_clone = ticker.clone();

        let handle = std::thread::Builder::new()
            .name(format!("vizza-{}", ticker))
            .spawn(move || {
                if let Err(e) = run_chart_window(&ticker_clone, historical_data_path, trade_rx) {
                    eprintln!("Chart error for {}: {}", ticker_clone, e);
                }
            })
            .expect("Failed to spawn chart thread");

        self.charts.insert(ticker, ChartInstance { trade_tx, handle });
        eprintln!("Opened chart for {}", ticker);
    }

    /// Send a trade update to a specific chart
    pub fn send_trade(&self, ticker: &str, trade: PlotTrade) {
        if let Some(instance) = self.charts.get(ticker) {
            let _ = instance.trade_tx.send(trade);
        }
    }

    /// Close a specific chart
    pub fn close_chart(&mut self, ticker: &str) {
        if let Some(instance) = self.charts.remove(ticker) {
            drop(instance.trade_tx); // Signals chart to close
            // Chart thread will exit when channel closes
            eprintln!("Closed chart for {}", ticker);
        }
    }

    /// Close all charts
    pub fn close_all(&mut self) {
        let tickers: Vec<_> = self.charts.keys().cloned().collect();
        for ticker in tickers {
            self.close_chart(&ticker);
        }
    }
}

/// Data source that receives trades from ibkr_proto backend
pub struct IBKRLiveSource {
    ticker: String,
    base_price: f64,
    trade_rx: Receiver<PlotTrade>,
    initialized: bool,
}

impl IBKRLiveSource {
    pub fn new(trade_rx: Receiver<PlotTrade>) -> Self {
        Self {
            ticker: String::new(),
            base_price: 0.0,
            trade_rx,
            initialized: false,
        }
    }
}

impl LiveDataSource for IBKRLiveSource {
    fn initialize(&mut self, historical_data: &LevelStore, ticker: &str) -> LiveResult<()> {
        self.ticker = ticker.to_string();

        if let Some(candles) = historical_data.get(60) {
            if let Some(last) = candles.last() {
                self.base_price = last.close as f64;
            }
        }

        self.initialized = true;
        eprintln!("Chart initialized for {}: base_price={:.2}", ticker, self.base_price);
        Ok(())
    }

    fn get_trades(&mut self, _now_ns: i64) -> Vec<PlotTrade> {
        // Drain all available trades
        self.trade_rx.try_iter().collect()
    }

    fn is_market_open(&self, _now_ns: i64) -> bool {
        true // IBKR stops sending when market closes
    }

    fn source_name(&self) -> &str {
        "IBKRLiveSource"
    }

    fn current_price(&self) -> f64 {
        self.base_price
    }
}

/// Run a vizza chart window
fn run_chart_window(
    ticker: &str,
    historical_data_path: String,
    trade_rx: Receiver<PlotTrade>,
) -> anyhow::Result<()> {
    // Create data source
    let ibkr_source = Box::new(IBKRLiveSource::new(trade_rx));

    // Run vizza
    PlotBuilder::new()
        .with_data_paths(vec![historical_data_path])
        .with_live_data(true, Some(100))
        .with_window_size(1200, 800)
        .run()?;

    Ok(())
}
```

### Step 3: Integrate with Backend

**TO DO**: Modify `ibkr_proto/src/ibkr/client.rs` to add chart management:

```rust
use crate::charts::ChartManager;

pub struct Backend {
    ui_rx: mpsc::Receiver<UiMessage>,
    ui_tx: mpsc::Sender<BackendMessage>,
    benchmark_tracker: BenchmarkTracker,
    execution_tracker: Option<Arc<ExecutionTracker>>,
    chart_manager: ChartManager,  // ← Add this
}

impl Backend {
    pub fn new(
        ui_rx: mpsc::Receiver<UiMessage>,
        ui_tx: mpsc::Sender<BackendMessage>,
        benchmark_tracker: BenchmarkTracker,
    ) -> Self {
        Self {
            ui_rx,
            ui_tx,
            benchmark_tracker,
            execution_tracker: None,
            chart_manager: ChartManager::new(),  // ← Add this
        }
    }

    pub async fn run(mut self) {
        // ... existing code ...

        // Track quotes by symbol for chart updates
        let mut quote_cache: HashMap<String, crate::ibkr::types::LiveQuote> = HashMap::new();

        while let Some(msg) = self.ui_rx.recv().await {
            match msg {
                // ... existing cases ...

                UiMessage::SetContract(contract) => {
                    // ... existing market data subscription code ...

                    // Forward quotes to chart if open
                    let symbol = contract.symbol.clone();
                    let chart_manager = &self.chart_manager;

                    // When we receive quotes, convert and send to chart
                    // This happens in the market data subscription handler
                }

                // New message types for chart control
                UiMessage::OpenChart { ticker, historical_path } => {
                    self.chart_manager.open_chart(ticker, historical_path);
                }

                UiMessage::CloseChart { ticker } => {
                    self.chart_manager.close_chart(&ticker);
                }
            }
        }
    }
}
```

### Step 4: Add Message Types

**TO DO**: Add to `ibkr_proto/src/channels/messages.rs`:

```rust
#[derive(Debug, Clone)]
pub enum UiMessage {
    // ... existing variants ...

    // Chart control
    OpenChart {
        ticker: String,
        historical_path: String,
    },
    CloseChart {
        ticker: String,
    },
}
```

### Step 5: Bridge Quotes to Charts

**TO DO**: Modify `ibkr_proto/src/ibkr/service.rs` to bridge quotes to charts:

```rust
pub async fn request_market_data(
    client: &Client,
    contract: &Contract,
    tx: mpsc::Sender<BackendMessage>,
    chart_manager: Arc<Mutex<ChartManager>>,  // ← Add parameter
) -> anyhow::Result<()> {
    let generic_ticks = &["233", "236", "258", "293"];
    let mut sub = client.market_data(contract, generic_ticks, false, false).await?;
    let contract = contract.clone();
    let symbol = contract.symbol.clone();

    tokio::spawn(async move {
        use time::OffsetDateTime;
        let mut quote = LiveQuote {
            contract_id: Some(contract.contract_id),
            symbol: contract.symbol.clone(),
            // ... other fields ...
        };

        let mut last_volume = 0i64;

        while let Some(evt) = sub.next().await {
            match evt {
                Ok(tick) => {
                    // ... existing tick processing ...

                    // Send quote to UI
                    let _ = tx.send(BackendMessage::Quote(quote.clone())).await;

                    // Convert to trade and send to chart
                    if let (Some(last_price), Some(volume)) = (quote.last, quote.volume) {
                        let volume_delta = volume - last_volume;
                        if volume_delta > 0 {
                            let trade = lod::PlotTrade {
                                ts: quote.updated_at.unix_timestamp_nanos() as i64,
                                price: last_price as f32,
                                size: volume_delta as f32,
                                flags: 0,
                            };

                            // Send to chart
                            if let Ok(manager) = chart_manager.lock() {
                                manager.send_trade(&symbol, trade);
                            }
                        }
                        last_volume = volume;
                    }
                }
                Err(e) => {
                    eprintln!("Market data error: {}", e);
                    break;
                }
            }
        }
    });

    Ok(())
}
```

### Step 6: Add UI Controls

**TO DO**: In your main UI (likely using egui), add chart controls:

```rust
// In your UI rendering code
ui.horizontal(|ui| {
    if ui.button("📊 Open Chart").clicked() {
        let _ = ui_tx.send(UiMessage::OpenChart {
            ticker: current_symbol.clone(),
            historical_path: format!("data/{}.nohlcv", current_symbol),
        });
    }

    if ui.button("❌ Close Chart").clicked() {
        let _ = ui_tx.send(UiMessage::CloseChart {
            ticker: current_symbol.clone(),
        });
    }
});
```

## Complete Example (Reference Implementation)

Here's how the integration would look once implemented:

```rust
// ibkr_proto/src/charts/mod.rs
use crossbeam_channel::{Receiver, Sender, unbounded};
use lod::PlotTrade;
use std::collections::HashMap;
use std::thread::JoinHandle;

pub struct ChartManager {
    charts: HashMap<String, ChartInstance>,
}

struct ChartInstance {
    trade_tx: Sender<PlotTrade>,
    handle: JoinHandle<()>,
}

impl ChartManager {
    pub fn new() -> Self {
        Self { charts: HashMap::new() }
    }

    pub fn open_chart(&mut self, ticker: String, data_path: String) {
        let (tx, rx) = unbounded();
        let ticker_clone = ticker.clone();

        let handle = std::thread::spawn(move || {
            let _ = vizza::PlotBuilder::new()
                .with_data_paths(vec![data_path])
                .with_live_data(true, Some(100))
                .run();
        });

        self.charts.insert(ticker, ChartInstance {
            trade_tx: tx,
            handle,
        });
    }

    pub fn send_trade(&self, ticker: &str, trade: PlotTrade) {
        if let Some(instance) = self.charts.get(ticker) {
            let _ = instance.trade_tx.send(trade);
        }
    }

    pub fn close_chart(&mut self, ticker: &str) {
        self.charts.remove(ticker);
    }
}

// In your backend's run loop:
impl Backend {
    pub async fn run(mut self) {
        while let Some(msg) = self.ui_rx.recv().await {
            match msg {
                UiMessage::OpenChart { ticker, historical_path } => {
                    self.chart_manager.open_chart(ticker, historical_path);
                }
                // ... handle other messages ...
            }
        }
    }
}
```

## Usage from ibkr_proto UI (Once Implemented)

```rust
// When user clicks "Open Chart" button:
ui_tx.send(UiMessage::OpenChart {
    ticker: "AAPL".to_string(),
    historical_path: "data/AAPL.nohlcv".to_string(),
}).await?;

// Charts appear in separate windows
// Automatically receive live updates from IBKR
```

## Advanced Features (Future Enhancements)

Once the basic integration is complete, these features could be added:

### 1. Multiple Charts for Same Ticker (Different Timeframes)

```rust
pub struct ChartInstance {
    ticker: String,
    timeframe: LodLevel,
    trade_tx: Sender<PlotTrade>,
    handle: JoinHandle<()>,
}

// Allow multiple charts per ticker
let chart_id = format!("{}_{:?}", ticker, timeframe);
self.charts.insert(chart_id, instance);
```

### 2. Chart State Persistence

```rust
// Save chart window position/size
impl ChartInstance {
    fn save_state(&self) -> ChartState {
        ChartState {
            position: self.window_position,
            size: self.window_size,
            zoom_level: self.zoom,
        }
    }
}
```

### 3. Synchronized Charts

```rust
// Link multiple charts to scroll together
pub struct ChartGroup {
    charts: Vec<String>,
    shared_time_range: Arc<Mutex<TimeRange>>,
}
```

### 4. Chart Presets

```rust
// Quick layouts
pub enum ChartLayout {
    Single,
    TwoByTwo,    // 4 charts
    ThreeColumn, // 3 charts side-by-side
}

impl ChartManager {
    pub fn open_layout(&mut self, layout: ChartLayout, tickers: Vec<String>) {
        // Open multiple charts in specific arrangement
    }
}
```

## Error Handling

```rust
impl ChartManager {
    pub fn open_chart(&mut self, ticker: String, data_path: String) -> Result<(), String> {
        // Check if historical data exists
        if !std::path::Path::new(&data_path).exists() {
            return Err(format!("Historical data not found: {}", data_path));
        }

        // Check for duplicate
        if self.charts.contains_key(&ticker) {
            return Err(format!("Chart for {} already open", ticker));
        }

        // Try to spawn chart
        let (tx, rx) = unbounded();
        let ticker_clone = ticker.clone();

        let handle = std::thread::Builder::new()
            .name(format!("chart-{}", ticker))
            .spawn(move || {
                if let Err(e) = run_chart(&ticker_clone, data_path, rx) {
                    eprintln!("Chart failed for {}: {}", ticker_clone, e);
                }
            })
            .map_err(|e| format!("Failed to spawn chart thread: {}", e))?;

        self.charts.insert(ticker, ChartInstance { trade_tx: tx, handle });
        Ok(())
    }
}
```

## Performance Considerations

### 1. Limit Active Charts

```rust
const MAX_CHARTS: usize = 10;

pub fn open_chart(&mut self, ticker: String, data_path: String) -> Result<()> {
    if self.charts.len() >= MAX_CHARTS {
        return Err("Maximum number of charts open".into());
    }
    // ...
}
```

### 2. Efficient Data Broadcasting

```rust
// Use broadcast channel if multiple charts need same data
use tokio::sync::broadcast;

let (tx, _rx) = broadcast::channel(1000);

// Each chart subscribes
let mut rx1 = tx.subscribe();
let mut rx2 = tx.subscribe();
```

### 3. Throttle Updates

```rust
// Don't send every single tick to charts
let mut last_update = Instant::now();
const UPDATE_INTERVAL: Duration = Duration::from_millis(100);

if last_update.elapsed() >= UPDATE_INTERVAL {
    chart_manager.send_trade(&symbol, trade);
    last_update = Instant::now();
}
```

## Lifecycle Management

```rust
impl Drop for ChartManager {
    fn drop(&mut self) {
        eprintln!("Closing all charts...");
        self.close_all();
    }
}

impl Backend {
    pub async fn run(mut self) {
        // ... main loop ...

        // When backend exits, charts are automatically closed
        drop(self.chart_manager);
    }
}
```

## Testing

Create a mock chart manager for testing:

```rust
pub struct MockChartManager {
    opened_charts: Vec<String>,
}

impl MockChartManager {
    pub fn open_chart(&mut self, ticker: String, _data_path: String) {
        self.opened_charts.push(ticker);
    }

    pub fn assert_chart_opened(&self, ticker: &str) {
        assert!(self.opened_charts.contains(&ticker.to_string()));
    }
}
```

## Implementation Checklist

To implement this integration, the following tasks need to be completed:

### Prerequisites
- [ ] Add vizza and lod dependencies to `ibkr_proto/Cargo.toml`
- [ ] Add crossbeam-channel dependency for thread communication

### Core Implementation
- [ ] Create `ibkr_proto/src/charts/mod.rs` with `ChartManager` struct
- [ ] Implement `IBKRLiveSource` that implements vizza's `LiveDataSource` trait
- [ ] Add chart control message types to `UiMessage` enum
- [ ] Add `chart_manager` field to `Backend` struct
- [ ] Initialize `ChartManager` in `Backend::new()`

### Integration Points
- [ ] Modify `request_market_data` to forward quotes to charts
- [ ] Add handler cases for `OpenChart` and `CloseChart` messages in Backend
- [ ] Bridge `BackendMessage::Quote` to `PlotTrade` conversion
- [ ] Add UI controls for opening/closing charts

### Testing
- [ ] Test single chart window opening
- [ ] Test multiple concurrent charts
- [ ] Test chart closing and cleanup
- [ ] Test quote data flow from IBKR to chart
- [ ] Test thread safety and resource cleanup

### Documentation
- [ ] Update this document to reflect actual implementation
- [ ] Add usage examples
- [ ] Document any limitations or known issues

## Summary

This document provides a comprehensive design for integrating vizza charts into ibkr_proto. While the integration hasn't been built yet, the architecture is sound and leverages existing capabilities of both systems:

**Why this design works:**
1. ✅ Charts run in separate threads (avoiding winit/tokio conflicts)
2. ✅ Channels bridge async backend to sync charts efficiently
3. ✅ ChartManager provides centralized window management
4. ✅ Minimal changes needed to existing ibkr_proto code
5. ✅ Charts can automatically receive live IBKR updates

**The integration will be straightforward because:**
- ibkr_proto already streams market data via `BackendMessage::Quote`
- vizza already has `LiveDataSource` trait for custom data sources
- Thread separation avoids event loop conflicts
- No complex state synchronization required

Once implemented, the main ibkr_proto application will remain largely unchanged, with charts being an additive feature that can be toggled on demand.
