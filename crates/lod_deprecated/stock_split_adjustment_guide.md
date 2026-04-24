# Stock Split Extraction and Price Adjustment Guide

This guide explains how stock splits are extracted from corporate action databases and applied to historical market data to create split-adjusted prices.

## Overview

Stock splits change the number of shares outstanding and proportionally affect the stock price. To compare historical prices accurately, we must adjust pre-split prices to reflect the cumulative effect of all subsequent splits.

**Key Principle**: Historical prices are divided by the cumulative split ratio to normalize them to the current share structure.

## Components

Existing split files are located in ../../db3/

### 1. Split Data Extraction (`src/splits.rs`, `src/bin/extract_splits.rs`)

#### Data Source
Splits are extracted from a SQLite database containing corporate actions:
- **Table**: `ACTIONS_Corporate_Actions`
- **Filter**: `action = 'split'`
- **Fields**: `ticker`, `date`, `value` (split ratio as string)

#### Data Structures

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockSplit {
    pub date: NaiveDate,        // When the split occurred
    pub ratio: f64,             // Multiplication factor (e.g., 10.0 for 10:1)
    pub description: String,    // Human-readable (e.g., "10:1 split")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MultiTickerSplitsData {
    pub tickers: HashMap<String, Vec<StockSplit>>,
}
```

#### Extraction Process

```rust
pub fn extract_splits_batch(
    db_path: &Path,
    tickers: &[String],
) -> Result<HashMap<String, Vec<StockSplit>>>
```

**Steps**:
1. Connect to SQLite database
2. Query `ACTIONS_Corporate_Actions` WHERE `action = 'split'` AND `ticker IN (...)`
3. Parse each row:
   - `date`: Parse YYYY-MM-DD string to `NaiveDate`
   - `ratio`: Parse string to `f64` (e.g., "10.0" → 10.0)
   - `description`: Generate from ratio (e.g., "10:1 split")
4. Group splits by ticker in a `HashMap<String, Vec<StockSplit>>`
5. Splits are ordered by date DESC (newest first) from the query

#### Split Ratio Interpretation

| Ratio | Type | Description | Effect on Shares | Effect on Price |
|-------|------|-------------|------------------|-----------------|
| 10.0 | Forward | 10:1 split | 10x more shares | Price ÷ 10 |
| 4.0 | Forward | 4:1 split | 4x more shares | Price ÷ 4 |
| 2.0 | Forward | 2:1 split | 2x more shares | Price ÷ 2 |
| 0.5 | Reverse | 1:2 reverse | Half the shares | Price × 2 |
| 0.1 | Reverse | 1:10 reverse | 1/10 shares | Price × 10 |

### 2. Cumulative Adjustment Calculation

The core algorithm for computing price adjustments:

```rust
pub fn calculate_cumulative_adjustment(
    splits: &[StockSplit],
    as_of_date: NaiveDate
) -> f64
```

#### Algorithm Logic

For a given historical date, we need to find the **divisor** to apply to prices from that date.

**Rule**: Multiply the ratios of all splits that occurred AFTER the `as_of_date`.

**Why?**
- Splits that happened after our date increased the share count
- To compare apples-to-apples, we divide old prices by this factor
- Splits that happened on or before our date don't affect our price (we're already "post-split" for those)

**Example Walkthrough**:

Given splits:
```rust
[
    StockSplit { date: 2024-06-10, ratio: 10.0, description: "10:1 split" },
    StockSplit { date: 2021-07-20, ratio: 4.0,  description: "4:1 split"  },
]
```

Adjustments for different dates:
- **Date: 2020-01-01** (before both splits)
  - Both splits occurred after → divisor = 10.0 × 4.0 = 40.0
  - A $100 price becomes $100 / 40.0 = $2.50 (split-adjusted)

- **Date: 2022-01-01** (after 2021 split, before 2024 split)
  - Only 2024 split occurred after → divisor = 10.0
  - A $100 price becomes $100 / 10.0 = $10.00 (split-adjusted)

- **Date: 2025-01-01** (after both splits)
  - No splits occurred after → divisor = 1.0
  - A $100 price remains $100 (no adjustment needed)

#### Implementation

```rust
pub fn calculate_cumulative_adjustment(splits: &[StockSplit], as_of_date: NaiveDate) -> f64 {
    let mut adjustment = 1.0;

    for split in splits {
        if split.date <= as_of_date {
            // Split happened on or before our date → already incorporated
            continue;
        } else {
            // Split happened after our date → need to divide by this ratio
            adjustment *= split.ratio;
        }
    }

    adjustment
}
```

**Test Case** (from `src/splits.rs`):
```rust
#[test]
fn test_cumulative_adjustment() {
    let splits = vec![
        StockSplit { date: 2024-06-10, ratio: 10.0, description: "10:1 split" },
        StockSplit { date: 2021-07-20, ratio: 4.0,  description: "4:1 split"  },
    ];

    assert_eq!(calculate_cumulative_adjustment(&splits, 2020-01-01), 40.0);  // Before both
    assert_eq!(calculate_cumulative_adjustment(&splits, 2022-01-01), 10.0);  // After 2021, before 2024
    assert_eq!(calculate_cumulative_adjustment(&splits, 2025-01-01), 1.0);   // After both
}
```

### 3. Applying Adjustments to OHLCV Data

**Use Case**: Adjusting historical OHLCV (Open, High, Low, Close, Volume) bars for charting and analysis.

**Source**: `src/bin/plot_ohlcv_with_split_adjust.rs`

#### Data Structure
```rust
struct OhlcvBar {
    timestamp: DateTime<Utc>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: u64,  // Note: Volume is NOT adjusted
}
```

#### Application Function

```rust
fn apply_split_adjustments(bars: &mut [OhlcvBar], splits: &[StockSplit]) {
    for bar in bars.iter_mut() {
        let adjustment = calculate_cumulative_adjustment(splits, bar.timestamp.date_naive());
        if adjustment != 1.0 {
            // Divide prices by adjustment factor to normalize pre-split prices
            bar.open /= adjustment;
            bar.high /= adjustment;
            bar.low /= adjustment;
            bar.close /= adjustment;
            // Note: Volume is typically NOT adjusted for splits
        }
    }
}
```

**Important**: Volume is usually NOT adjusted because:
- Volume represents actual shares traded on that day
- For analysis, you may want actual historical volume, not hypothetical adjusted volume
- Some systems do adjust volume (multiply by split ratio), but it's not universal

#### Complete Workflow

1. **Extract splits** from database:
   ```bash
   cargo run --bin extract_splits -- \
       --db ./data/corporate/sharadar_data.sqlite \
       --ticker NVDA,AAPL,TSLA \
       --output ./splits_output.json
   ```

2. **Load and apply** to OHLCV data:
   ```rust
   // Read OHLCV data from custom binary file (.ntrd, .nbbo, or .nohlcv)
   

   // Load splits from JSON
   let splits_data = load_splits_from_json(splits_path)?;

   // Apply adjustments
   apply_split_adjustments(&mut bars, &splits_data.splits);

   // Now bars contain split-adjusted prices
   ```

3. **Visualize**:


### 4. Integration with EMA and Other Indicators

**Current State**: The EMA calculator (`src/bin/ema_calculator_mmap_rayon.rs`) does NOT currently apply split adjustments.

**To Integrate Splits into EMA Calculation**:

```rust
// 1. Load splits for the ticker
let splits = load_splits_from_json(splits_path)?;

// 2. As you process each trade, apply adjustment
while let Some(rec) = decoder.decode_record_ref()? {
    if let Some(tr) = rec.get::<TradeMsg>() {
        let raw_price = price_to_dollars(tr.price);
        let trade_date = DateTime::<Utc>::from_timestamp_nanos(tr.hd.ts_event as i64)
            .date_naive();

        // Apply split adjustment
        let adjustment = calculate_cumulative_adjustment(&splits, trade_date);
        let adjusted_price = raw_price / adjustment;

        // Use adjusted_price for EMA calculation
        ema_calculator.add_value(adjusted_price);
    }
}
```

**Trade-offs**:
- **Adjusted prices**: Better for long-term trend analysis across splits
- **Unadjusted prices**: Represents actual traded prices; needed for some strategies
- **Decision**: Depends on your use case. For backtesting strategies across splits, use adjusted prices.

### 5. File I/O Operations

#### Saving Splits to JSON

```rust
pub fn save_multi_ticker_splits_to_json(
    splits_data: &MultiTickerSplitsData,
    output_path: &Path,
) -> Result<()>
```

Creates a JSON file with format (see `corporate_data_formats.md` for details):
```json
{
  "tickers": {
    "NVDA": [
      { "date": "2024-06-10", "ratio": 10.0, "description": "10:1 split" },
      { "date": "2021-07-20", "ratio": 4.0,  "description": "4:1 split"  }
    ],
    "AAPL": [
      { "date": "2020-08-31", "ratio": 4.0, "description": "4:1 split" }
    ]
  }
}
```

#### Loading Splits from JSON

```rust
pub fn load_splits_from_json(path: &Path) -> Result<SplitsData>
```

Deserializes the JSON file into `SplitsData` or `MultiTickerSplitsData` structures.

### 6. Common Pitfalls and Best Practices

#### Pitfalls
1. **Wrong direction**: Multiplying instead of dividing (or vice versa)
   - Remember: Divide historical prices by the adjustment factor
   - The adjustment is the product of future split ratios

2. **Forgetting to sort**: If splits aren't chronologically ordered, the algorithm still works, but it's clearer if they are

3. **Adjusting volume**: Standard practice is to NOT adjust volume, but some systems do
   - Be consistent with your data provider's convention

4. **Ex-date vs. effective date**: Corporate actions have multiple dates
   - This implementation uses the "date" field from the database
   - Verify this matches your provider's convention (usually ex-date)

#### Best Practices
1. **Cache splits**: Load splits once per ticker, not per data point
2. **Pre-compute for date ranges**: If processing many bars, you could pre-compute adjustment factors for date buckets
3. **Validate with charts**: Compare your adjusted prices with known sources (Yahoo Finance, Bloomberg)
4. **Document your choice**: Be explicit about whether your prices are adjusted or unadjusted

### 7. Performance Considerations

#### Current Implementation
- **Extraction**: Single query with `IN (...)` clause for batch extraction
- **Adjustment**: O(n × m) where n = number of bars, m = number of splits per ticker
  - For most stocks, m is small (< 10), so this is acceptable

#### Optimization Opportunities
If you need to process many tickers with many bars:

1. **Pre-compute date → adjustment map**:
   ```rust
   // For each unique date in your dataset
   let adjustment_map: HashMap<NaiveDate, f64> = dates
       .into_iter()
       .map(|date| (date, calculate_cumulative_adjustment(&splits, date)))
       .collect();

   // Then lookup is O(1) instead of O(m)
   ```

2. **Bucket by date range**:
   - If splits are [split1: 2020, split2: 2024], create date ranges
   - Range 1 (before 2020): divisor = ratio1 × ratio2
   - Range 2 (2020-2024): divisor = ratio2
   - Range 3 (after 2024): divisor = 1.0
   - Binary search to find the right range for each bar


## Summary

**Workflow**:
1. Extract splits from SQLite database → JSON file
2. Load JSON file into `Vec<StockSplit>`
3. For each historical data point (trade, bar, etc.):
   - Call `calculate_cumulative_adjustment(splits, date)`
   - Divide price by adjustment factor
4. Use adjusted prices for analysis, charting, and indicator calculation

**Key Functions**:
- `extract_splits_batch()` - Extract from database
- `calculate_cumulative_adjustment()` - Compute adjustment factor
- `apply_split_adjustments()` - Apply to OHLCV data
- `save_multi_ticker_splits_to_json()` / `load_splits_from_json()` - Persistence

**Files**:
- `src/splits.rs` - Core logic
- `src/bin/extract_splits.rs` - Extraction tool
- `src/bin/plot_ohlcv_with_split_adjust.rs` - Application example
- `corporate_data_formats.md` - JSON schema documentation
