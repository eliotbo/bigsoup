# Live Data API Documentation

## Overview

Vizza's Live Data API provides a pluggable architecture for real-time market data visualization. The new implementation uses actual historical patterns instead of naive random walks, providing realistic synthetic data that seamlessly continues from historical bars.

## Quick Start

### Basic Usage

```rust
use vizza::PlotBuilder;

PlotBuilder::new()
    .with_data_paths(vec!["path/to/data.nohlcv".to_string()])
    .with_live_data(true, Some(100))  // Enable live data, 100ms updates
    .with_auto_y_scale(true)           // Auto-scale to fit live data
    .run()?;
```

### What You Get

✅ **Real Base Price**: Live data starts from the actual last closing price
✅ **Calculated Volatility**: Uses 20-bar log returns (not hardcoded 2%)
✅ **Market Hours**: Respects US equity hours (9:30-16:00 ET)
✅ **Weekend Awareness**: No trades on Saturday/Sunday
✅ **Holiday Detection**: Identifies market closures (gaps > 3 days)
✅ **Smooth Transition**: Seamless continuation from historical data

## Architecture

### LiveDataSource Trait

The core abstraction that allows pluggable data sources:

```rust
pub trait LiveDataSource: Send + Sync {
    /// Initialize with historical context
    fn initialize(&mut self, historical_data: &LevelStore, ticker: &str) -> Result<()>;

    /// Generate/fetch trades up to current time
    fn get_trades(&mut self, now_ns: i64) -> Vec<PlotTrade>;

    /// Check if market is open
    fn is_market_open(&self, now_ns: i64) -> bool;

    /// Get source metadata
    fn source_name(&self) -> &str;

    /// Get current base price
    fn current_price(&self) -> f64;
}
```

### Built-in Sources

#### 1. SyntheticDataSource (Default)

Smart synthetic data using historical patterns:

#### 2. Real Live Data Sources

See [IBKR_INTEGRATION.md](IBKR_INTEGRATION.md) for integrating Interactive Brokers live data.

**Quick example**:
```rust
// Implement LiveDataSource for IBKR
struct IBKRDataSource { /* ... */ }

impl LiveDataSource for IBKRDataSource {
    fn initialize(&mut self, historical_data: &LevelStore, ticker: &str) -> Result<()> {
        // Connect to IBKR TWS/Gateway
        // Subscribe to tick-by-tick data
    }

    fn get_trades(&mut self, now_ns: i64) -> Vec<PlotTrade> {
        // Return buffered trades from IBKR
    }
    // ...
}

// Use it
let ibkr = Box::new(IBKRDataSource::new("127.0.0.1", 7496, 1));
let live_manager = LiveDataManager::with_data_source(level_store, ibkr, "AAPL");
```

See `examples/ibkr_live_example.rs` for complete implementation.

#### SyntheticDataSource Usage

```rust
use vizza::live::SyntheticDataSource;
use vizza::live_view::LiveDataManager;

let mut source = SyntheticDataSource::new();
// Automatically used by default in LiveDataManager
```

**Features:**
- Extracts last closing price from historical data
- Calculates volatility from recent price movements
- Respects market calendar (hours, weekends, holidays)
- Generates realistic trade flow using TradeSimulator

**Initialization Process:**
1. Reads last bar from 60-second historical data
2. Extracts `base_price = last_bar.close`
3. Calculates volatility: `σ = std(log_returns) * √252`
4. Sets up TradeSimulator with real parameters
5. Configures market calendar for US equity hours

#### 2. Custom Sources (Future)

Planned implementations:
- `WebSocketSource`: Real-time exchange feeds
- `FileTailSource`: Replay from file
- `RestApiSource`: Polling-based data

## Creating Custom Data Sources

### Example: Simple Test Pattern

```rust
use vizza::live::{LiveDataSource, LiveDataError, Result as LiveResult};
use lod::{LevelStore, PlotTrade};

struct SineWaveSource {
    base_price: f64,
    start_time_ns: i64,
}

impl LiveDataSource for SineWaveSource {
    fn initialize(&mut self, historical_data: &LevelStore, _ticker: &str) -> LiveResult<()> {
        if let Some(candles) = historical_data.get(60) {
            if let Some(last) = candles.last() {
                self.base_price = last.close as f64;
                self.start_time_ns = last.ts;
                return Ok(());
            }
        }
        Err(LiveDataError::InvalidHistoricalData)
    }

    fn get_trades(&mut self, now_ns: i64) -> Vec<PlotTrade> {
        let elapsed_s = (now_ns - self.start_time_ns) as f64 / 1e9;
        let price = self.base_price + (elapsed_s.sin() * 5.0);

        vec![PlotTrade {
            ts: now_ns,
            price: price as f32,
            size: 100.0,
            flags: 0,
        }]
    }

    fn is_market_open(&self, _now_ns: i64) -> bool {
        true
    }

    fn source_name(&self) -> &str {
        "SineWaveSource"
    }

    fn current_price(&self) -> f64 {
        self.base_price
    }
}
```

### Using Custom Sources

```rust
use vizza::live_view::LiveDataManager;
use std::sync::{Arc, Mutex};

let level_store = Arc::new(Mutex::new(LevelStore::new()));
let custom_source = Box::new(SineWaveSource {
    base_price: 100.0,
    start_time_ns: 0,
});

let live_manager = LiveDataManager::with_data_source(
    level_store,
    custom_source,
    "TEST",
);
```

## Technical Details

### Volatility Calculation

The `SyntheticDataSource` calculates annualized volatility using:

```rust
// 1. Compute log returns
for i in 1..candles.len() {
    let log_return = (candles[i].close / candles[i-1].close).ln();
    returns.push(log_return);
}

// 2. Calculate standard deviation
let mean = returns.iter().sum() / returns.len();
let variance = returns.iter()
    .map(|r| (r - mean).powi(2))
    .sum() / returns.len();
let std_dev = variance.sqrt();

// 3. Annualize (assuming daily bars)
let volatility = std_dev * (252.0_f64).sqrt();
```

### Market Calendar

The `MarketCalendar` checks:

1. **Weekends**: `Saturday | Sunday → market closed`
2. **Trading Hours**: `9:30 AM - 4:00 PM ET`
3. **Holidays**: Gap detection `(current_time - last_bar) > 3 days`

```rust
pub fn is_market_open(&self, timestamp_ns: i64) -> bool {
    let dt = DateTime::from_timestamp(timestamp_ns / 1_000_000_000, 0)?;

    if self.is_weekend(&dt) {
        return false;
    }

    let time = dt.time();
    time >= market_open && time < market_close
}
```

### Price Continuity

The system ensures smooth transition:

```rust
// Historical data ends at bar N
let last_historical_bar = historical_data.last();
let base_price = last_historical_bar.close;

// Live data starts at bar N+1
let next_bar_time = last_historical_bar.ts + interval_ns;

// Price continues from actual close, not arbitrary value
simulator.initialize(base_price, calculated_volatility, next_bar_time);
```

## Examples

### Example 1: Basic Live Data

```rust
use vizza::PlotBuilder;

PlotBuilder::new()
    .with_data_paths(vec!["data.nohlcv".to_string()])
    .with_live_data(true, Some(100))
    .run()?;
```

### Example 2: Custom Update Interval

```rust
PlotBuilder::new()
    .with_data_paths(vec!["data.nohlcv".to_string()])
    .with_live_data(true, Some(50))   // 50ms updates (faster)
    .with_auto_y_scale(true)
    .run()?;
```

### Example 3: Multi-Viewport Live

```rust
PlotBuilder::new()
    .with_data_paths(vec![
        "fast_data.nohlcv".to_string(),
        "slow_data.nohlcv".to_string(),
    ])
    .with_grid(1, 2)
    .with_live_data(true, Some(100))
    .run()?;
```

### Example 4: Run the Showcase

```bash
cargo run --example live_data_showcase
```

## API Reference

### LiveDataManager

```rust
impl LiveDataManager {
    /// Create with default SyntheticDataSource
    pub fn new(level_store: Arc<Mutex<LevelStore>>) -> Self;

    /// Create with custom data source
    pub fn with_data_source(
        level_store: Arc<Mutex<LevelStore>>,
        data_source: Box<dyn LiveDataSource>,
        ticker: &str,
    ) -> Self;

    /// Update live engine (call each frame)
    pub fn update(&mut self, dt: Duration);

    /// Prepare data for rendering
    pub fn prepare_render(&mut self, interval_secs: u64) -> LiveSnapshotState;
}
```

### PlotBuilder

```rust
impl PlotBuilder {
    /// Enable live data with update interval
    pub fn with_live_data(self, enabled: bool, update_interval_ms: Option<u64>) -> Self;
}
```

## Performance

- **Synthetic Generation**: < 1ms per update
- **Memory**: Minimal allocation per tick
- **Update Rate**: 50-100ms recommended
- **Scalability**: Supports multiple viewports

## Migration Guide

### Before (Old API)

```rust
// Hardcoded values
let simulator = TradeSimulator::new(
    "SIM".to_string(),
    100.0,        // ❌ Hardcoded base price
    0.02,         // ❌ Hardcoded 2% volatility
    120.0,
    base_epoch_ns,
    0xC0FFEE,
);
```

### After (New API)

```rust
// Smart defaults from historical data
let mut source = SyntheticDataSource::new();
source.initialize(&level_store, "AAPL")?;
// ✅ Extracts real base price
// ✅ Calculates actual volatility
// ✅ Respects market hours
```

## Future Enhancements

### Phase 3: External Data Sources

**WebSocket Source:**
```rust
pub struct WebSocketSource {
    url: String,
    reconnect_strategy: ReconnectStrategy,
    buffer: RingBuffer<PlotTrade>,
}
```

**File Replay Source:**
```rust
pub struct FileTailSource {
    file_path: PathBuf,
    parser: TradeParser,
}
```

### Advanced Features

- Multi-ticker support with different sources per viewport
- Options chain integration with Greeks calculation
- News event injection for volatility simulation
- Advanced volume profiling (time-of-day patterns)

## Troubleshooting

**Q: Live data jumps to a different price**
A: This shouldn't happen with the new API. Check that historical data is loaded correctly.

**Q: No live trades appearing**
A: ~~Check if current time is during market hours.~~ **UPDATE**: As of the latest fix, `SyntheticDataSource` is lenient about market hours and only blocks on weekends. If you still see no trades:
- Check console for "✓ SyntheticDataSource initialized" message
- Verify data gap is < 7 days (larger gaps are treated as holidays)
- Ensure current day is not Saturday or Sunday
- See [BUGFIX_LIVE_DATA.md](BUGFIX_LIVE_DATA.md) for details on the recent fix

**Q: Volatility seems wrong / Price swings too much**
A: The volatility is automatically scaled for intraday simulation. If you see excessive swings:
- Check console for "Volatility: raw=X, scaled=Y" message
- Scaled volatility should be 0.1%-2% for typical stocks
- See [VOLATILITY_FIX.md](VOLATILITY_FIX.md) for customization options
- Note: Annualized volatility is scaled by 0.01 (1%) to prevent compounding effects

**Q: How to disable market hours check completely?**
A: Create a custom LiveDataSource with `is_market_open()` always returning `true`.

**Q: What's the difference between "market closed" and "holiday"?**
A:
- **Weekend**: Saturday/Sunday, blocked by weekday check
- **Holiday**: Gap > 7 days from last bar, indicates major market closure
- **After hours**: No longer blocked for synthetic data (visualization purposes)

## See Also

- [LIVE_DATA_API_PLAN.md](LIVE_DATA_API_PLAN.md) - Implementation plan and status
- [examples/live_data_showcase.rs](examples/live_data_showcase.rs) - Comprehensive examples
- [examples/plot_builder_demo.rs](examples/plot_builder_demo.rs) - General usage examples
