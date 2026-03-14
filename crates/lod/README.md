# LOD (Level-of-Detail) Crate

High-performance multi-resolution data aggregation library for time-series data.

## Features

- **Streaming Aggregation**: Process data in real-time with minimal memory overhead
- **Multi-Resolution Support**: Aggregate data at multiple time intervals simultaneously
- **Zero-Copy Windows**: Efficient data access without unnecessary allocations
- **Generic Design**: Works with any quote-like data (candles, NBBO, trades)
- **NBBO Decoder**: Built-in support for reading `.nbbo` binary files

## Phase 1 Implementation Complete

This implementation covers Phase 1 of the LOD subcrate plan:

1. ✅ **Crate Structure**: Created and integrated into workspace
2. ✅ **QuoteLike Trait**: Generic interface for time-series data
3. ✅ **LevelGenerator Trait**: Extensible aggregation framework
4. ✅ **Adapters**: Support for candles and NBBO records
5. ✅ **NBBO Decoder**: Standalone binary and library for `.nbbo` files

## Usage

### Basic Aggregation

```rust
use lod::{StreamingAggregator, LevelStore};
use lod::traits::SimpleCandle;

// Create test data
let candles = vec![
    SimpleCandle {
        timestamp: 0,
        open: 100.0,
        high: 101.0,
        low: 99.0,
        close: 100.5,
        volume: 1000.0,
    },
    // ... more candles
];

// Create aggregator with 5s, 10s, and 30s intervals
let mut aggregator = StreamingAggregator::new(1, vec![5, 10, 30]);

// Process candles
for candle in &candles {
    aggregator.push(candle);
}

// Get aggregated levels
let batch = aggregator.seal();
let store = LevelStore::from_stream(batch.levels.into_iter().collect());
```

### NBBO Decoder

Use the standalone binary to decode `.nbbo` files:

```bash
# Show file information
cargo run --bin nbbo_decoder -- file.nbbo info

# Calculate statistics
cargo run --bin nbbo_decoder -- file.nbbo stats

# View first 10 records
cargo run --bin nbbo_decoder -- file.nbbo head 10

# Export as JSON
cargo run --bin nbbo_decoder -- file.nbbo export > output.json
```

## Architecture

- **traits.rs**: Core trait definitions (`QuoteLike`, `LevelGenerator`)
- **aggregator.rs**: Streaming aggregation engine
- **levels.rs**: Level storage and metadata
- **window.rs**: Zero-copy window extraction utilities
- **nbbo_decoder.rs**: Complete NBBO file decoder

## Next Steps

Future phases will implement:
- Phase 2: Advanced streaming aggregator features
- Phase 3: Enhanced caching infrastructure
- Phase 4: Additional window utilities
- Phase 5: Comprehensive benchmarks
- Phase 6: Full integration with viz crate