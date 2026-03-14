# Dividend Extraction and Analysis Guide

This guide explains how dividends are extracted from corporate action databases, stored in an efficient binary format, and used for dividend yield analysis and income calculations.

## Overview

Dividends represent cash payments from companies to shareholders. This pipeline extracts dividend history from a database, stores it in a compact binary format with fast lookup capabilities, and provides utilities for analysis.

**Key Features**:
- Batch extraction from SQLite database
- Compact binary storage with indexing for fast lookups
- Single-ticker O(1) retrieval without loading entire file
- Dividend yield and annual income calculations
- Optional JSON export for debugging/compatibility

## Components

### 1. Dividend Data Extraction (`src/dividends.rs`, `src/bin/extract_dividends.rs`)

#### Data Source
Dividends are extracted from a SQLite database containing corporate actions:
- **Table**: `ACTIONS_Corporate_Actions`
- **Filter**: `action = 'dividend'`
- **Fields**: `ticker`, `date`, `value` (dividend amount as string)
- **Order**: By ticker, then date DESC (newest first)

#### Data Structures

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dividend {
    pub date: NaiveDate,    // Payment/ex-dividend date
    pub amount: f64,        // Dividend amount in dollars
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MultiTickerDividendsData {
    pub tickers: HashMap<String, Vec<Dividend>>,
}
```

#### Extraction Process

```rust
pub fn extract_dividends_batch(
    db_path: &Path,
    tickers: &[String],
) -> Result<HashMap<String, Vec<Dividend>>>
```

**Steps**:
1. Connect to SQLite database
2. Build parameterized query: `WHERE ticker IN (?, ?, ...) AND action = 'dividend'`
3. Parse each row:
   - `date`: Parse YYYY-MM-DD string to `NaiveDate`
   - `amount`: Parse string to `f64` (e.g., "0.25" → 0.25)
4. Group dividends by ticker in `HashMap<String, Vec<Dividend>>`
5. Results ordered by date DESC (most recent first)

**Example Query**:
```sql
SELECT ticker, date, value
FROM ACTIONS_Corporate_Actions
WHERE ticker IN ('AAPL', 'MSFT', 'JNJ') AND action = 'dividend'
ORDER BY ticker, date DESC
```

### 2. Binary File Format (.divbin)

The primary storage format is a custom binary format optimized for:
- **Compactness**: ~8-16 bytes per dividend record
- **Fast indexing**: O(1) single-ticker lookup
- **Sequential reading**: Efficient full-file iteration

See `corporate_data_formats.md` for complete format specification.

#### File Structure Overview

```
[Header: 18 bytes]
  - Magic bytes: "DIVD" (4 bytes)
  - Version: u16 (2 bytes)
  - Ticker count: u32 (4 bytes)
  - Index offset: u64 (8 bytes)

[Data Section: Variable size]
  - For each ticker:
    - Ticker (8 bytes, null-padded)
    - Dividend count (4 bytes)
    - Dividend records (8 bytes each):
      - Date as days since epoch (4 bytes)
      - Amount in cents (4 bytes)

[Index Section: 24 bytes per ticker]
  - For each ticker:
    - Ticker (8 bytes, null-padded)
    - File offset (8 bytes)
    - Record count (4 bytes)
    - Padding (4 bytes)
```

#### Binary Conversion

**To Binary** (`src/dividends.rs:52`):
```rust
impl Dividend {
    fn to_binary(&self) -> BinaryDividend {
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let days = (self.date - epoch).num_days() as u32;
        let cents = (self.amount * 100.0).round() as u32;

        BinaryDividend {
            date_days: days,
            amount_cents: cents,  // Stores as cents to avoid float precision
        }
    }
}
```

**From Binary** (`src/dividends.rs:63`):
```rust
impl Dividend {
    fn from_binary(binary: &BinaryDividend) -> Self {
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let date = epoch + chrono::Duration::days(binary.date_days as i64);
        let amount = binary.amount_cents as f64 / 100.0;

        Dividend { date, amount }
    }
}
```

**Why cents instead of float?**
- Avoids floating-point precision issues (0.1 + 0.2 ≠ 0.3)
- Dividends are typically specified to 2 decimal places (cents)
- Integer storage is exact and compact

### 3. Saving to Binary Format

**Function**: `save_dividends_to_binary()` (`src/dividends.rs:123`)

**Algorithm**:
1. Write header with placeholder index offset
2. For each ticker:
   - Record current file position (for index)
   - Write ticker (8 bytes, null-padded)
   - Write dividend count (4 bytes)
   - Write all dividend records (8 bytes each)
   - Store index entry in memory
3. Record index section position
4. Write all index entries
5. **Seek back** to header and update index offset
6. Flush to disk

**Why write index at the end?**
- We don't know the index offset until data section is complete
- Index allows fast O(1) lookup without scanning entire file
- Trade-off: Two writes to header (initial + update), but only one pass through data

**Usage**:
```bash
cargo run --bin extract_dividends -- \
    --db ./data/corporate/sharadar_data.sqlite \
    --ticker AAPL,MSFT,JNJ,PG,KO \
    --output ./dividends.divbin
```

**Output**:
```
Extracting dividends for 5 tickers from ./data/corporate/sharadar_data.sqlite
Tickers: AAPL, MSFT, JNJ, PG, KO
=====================================

AAPL: 87 dividend payments
  Total dividends: $21.75
  Recent dividends (last 4):
    2024-08-12 - $0.25
    2024-05-13 - $0.25
    2024-02-12 - $0.24
    2023-11-13 - $0.24
  Average dividend: $0.2500
  Estimated annual dividend (based on last 4): $0.98

[... more tickers ...]

=====================================
SUMMARY:
  Tickers with dividends: 5/5
  Total dividend payments: 347
  Sum of all dividends: $127.38

Binary file saved to: ./dividends.divbin
File size: 3.42 KB (3504 bytes)

Tip: Use 'read_dividends' tool to read this binary file
```

### 4. Loading from Binary Format

#### Load All Tickers

**Function**: `load_dividends_from_binary()` (`src/dividends.rs:216`)

```rust
pub fn load_dividends_from_binary(path: &Path)
    -> Result<HashMap<String, Vec<Dividend>>>
```

**Algorithm**:
1. Read and validate header (magic bytes, version)
2. Seek to index section (at `header.index_offset`)
3. Read all index entries into memory
4. For each index entry:
   - Seek to ticker's data offset
   - Read ticker name (8 bytes)
   - Read dividend count (4 bytes)
   - Read all dividend records
   - Convert from binary format
   - Insert into HashMap

**Performance**: Reads entire file, O(n) where n = total dividends

#### Load Single Ticker (Optimized)

**Function**: `load_single_ticker_from_binary()` (`src/dividends.rs:278`)

```rust
pub fn load_single_ticker_from_binary(path: &Path, ticker: &str)
    -> Result<Option<Vec<Dividend>>>
```

**Algorithm**:
1. Read header
2. Seek to index section
3. **Scan index entries** for matching ticker (O(m) where m = number of tickers)
4. If found:
   - Seek to ticker's data offset
   - Read only that ticker's dividends
   - Return Some(dividends)
5. If not found, return None

**Performance**: Only reads header + index + one ticker's data
- Much faster than loading entire file for single ticker
- Index scan is fast (24 bytes per ticker, sequential read)

**Usage**:
```bash
# Read all tickers
cargo run --bin read_dividends -- --input ./dividends.divbin

# Read single ticker
cargo run --bin read_dividends -- --input ./dividends.divbin --ticker AAPL

# Summary view
cargo run --bin read_dividends -- --input ./dividends.divbin --summary

# With annual breakdown
cargo run --bin read_dividends -- --input ./dividends.divbin --ticker AAPL --annual
```

### 5. Dividend Analysis Functions

#### Calculate Total Dividends

**Function**: `calculate_total_dividends()` (`src/dividends.rs:359`)

```rust
pub fn calculate_total_dividends(
    dividends: &[Dividend],
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
) -> f64
```

Sums dividend amounts within an optional date range.

**Examples**:
```rust
// Total all dividends
let total = calculate_total_dividends(&dividends, None, None);

// Dividends in 2024
let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
let total_2024 = calculate_total_dividends(&dividends, Some(start), Some(end));

// Dividends since IPO date
let ipo_date = NaiveDate::from_ymd_opt(2010, 1, 1).unwrap();
let total_since_ipo = calculate_total_dividends(&dividends, Some(ipo_date), None);
```

#### Calculate Annual Dividend Yield

**Function**: `calculate_annual_dividend_yield()` (`src/dividends.rs:375`)

```rust
pub fn calculate_annual_dividend_yield(dividends: &[Dividend], year: i32) -> f64
```

Returns total dividends paid in a specific calendar year.

**Example**:
```rust
let dividends = load_single_ticker_from_binary(path, "AAPL")?.unwrap();

for year in 2020..=2024 {
    let annual_div = calculate_annual_dividend_yield(&dividends, year);
    println!("{}: ${:.2}", year, annual_div);
}

// Output:
// 2020: $3.28
// 2021: $3.44
// 2022: $3.68
// 2023: $3.88
// 2024: $4.00
```

### 6. Integration with Trading Strategies

#### Use Case: Dividend Capture Strategy

A strategy that buys stocks before ex-dividend date to capture the dividend.

```rust
// Load dividend data
let dividends = load_single_ticker_from_binary(dividends_path, "AAPL")?
    .ok_or_else(|| anyhow::anyhow!("Ticker not found"))?;

// Find next dividend date
let today = chrono::Local::now().date_naive();
let next_dividend = dividends.iter()
    .find(|d| d.date > today)
    .ok_or_else(|| anyhow::anyhow!("No future dividends"))?;

println!("Next AAPL dividend: {} - ${:.2}", next_dividend.date, next_dividend.amount);

// Check if we should enter position (e.g., 5 days before ex-date)
let entry_date = next_dividend.date - chrono::Duration::days(5);
if today >= entry_date && today < next_dividend.date {
    println!("Enter position for dividend capture");
}
```

#### Use Case: Dividend Yield Screening

Find stocks with high dividend yields.

```rust
// Load all tickers
let all_dividends = load_dividends_from_binary(dividends_path)?;
let current_prices = load_current_prices()?; // Your price data

// Calculate yields
let mut yields: Vec<(String, f64)> = all_dividends
    .iter()
    .filter_map(|(ticker, divs)| {
        let annual_div = calculate_annual_dividend_yield(divs, 2024);
        let price = current_prices.get(ticker)?;
        let yield_pct = (annual_div / price) * 100.0;
        Some((ticker.clone(), yield_pct))
    })
    .collect();

// Sort by yield descending
yields.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

// Top 10 yielders
for (ticker, yield_pct) in yields.iter().take(10) {
    println!("{}: {:.2}%", ticker, yield_pct);
}
```

#### Use Case: Backtest with Dividend Reinvestment

Account for dividends when backtesting a buy-and-hold strategy.

```rust
struct Position {
    shares: f64,
    cost_basis: f64,
}

fn backtest_with_dividends(
    ticker: &str,
    initial_investment: f64,
    start_date: NaiveDate,
    end_date: NaiveDate,
    prices: &HashMap<NaiveDate, f64>,
    dividends: &[Dividend],
) -> f64 {
    // Initial purchase
    let entry_price = prices[&start_date];
    let mut position = Position {
        shares: initial_investment / entry_price,
        cost_basis: initial_investment,
    };

    // Process dividends in chronological order
    let mut divs_in_period: Vec<_> = dividends.iter()
        .filter(|d| d.date >= start_date && d.date <= end_date)
        .collect();
    divs_in_period.reverse(); // Now oldest first

    for div in divs_in_period {
        // Cash received from dividend
        let div_cash = position.shares * div.amount;

        // Reinvest immediately at price on dividend date
        let reinvest_price = prices.get(&div.date)
            .or_else(|| find_next_trading_day_price(prices, div.date))
            .unwrap();

        let new_shares = div_cash / reinvest_price;
        position.shares += new_shares;
        position.cost_basis += div_cash;
    }

    // Final value
    let exit_price = prices[&end_date];
    let final_value = position.shares * exit_price;

    let total_return = (final_value / position.cost_basis - 1.0) * 100.0;
    total_return
}
```

### 7. File Format Comparison

#### Binary (.divbin) - Primary Format

**Advantages**:
- Compact: ~8-16 bytes per dividend (vs ~50-80 bytes JSON)
- Fast indexed lookup: O(1) single ticker access
- No parsing overhead: Direct memory mapping
- Exact precision: Stores cents as integers

**Disadvantages**:
- Not human-readable
- Requires specialized tools to inspect
- Platform-dependent (endianness, alignment)

**When to use**: Production systems, high-performance analysis, large datasets

#### JSON (.json) - Compatibility Format

**Function**: `save_multi_ticker_dividends_to_json()` (`src/dividends.rs:337`)

**Advantages**:
- Human-readable and editable
- Language-agnostic
- Easy debugging
- Version control friendly

**Disadvantages**:
- Large file size
- Slower parsing
- Float precision issues possible

**When to use**: Debugging, configuration, cross-platform interchange, manual inspection

**Example JSON**:
```json
{
  "tickers": {
    "AAPL": [
      { "date": "2024-08-12", "amount": 0.25 },
      { "date": "2024-05-13", "amount": 0.25 },
      { "date": "2024-02-12", "amount": 0.24 }
    ],
    "MSFT": [
      { "date": "2024-09-11", "amount": 0.75 },
      { "date": "2024-06-12", "amount": 0.75 }
    ]
  }
}
```

### 8. Command-Line Tools

#### extract_dividends

Extract dividends from database to binary file.

```bash
cargo run --bin extract_dividends -- \
    --db ./data/corporate/sharadar_data.sqlite \
    --ticker AAPL,MSFT,JNJ,PG,KO,PFE,XOM,CVX,JPM \
    --output ./dividends.divbin \
    --annual-summary
```

**Flags**:
- `--db`: Path to SQLite database
- `--ticker`: Comma-separated list of tickers
- `--output`: Output file path (.divbin recommended)
- `--annual-summary`: Show annual dividend totals (optional)

#### read_dividends

Read and analyze dividend binary files.

```bash
# Read all tickers (detailed)
cargo run --bin read_dividends -- --input ./dividends.divbin

# Read specific ticker
cargo run --bin read_dividends -- --input ./dividends.divbin --ticker AAPL

# Summary table
cargo run --bin read_dividends -- --input ./dividends.divbin --summary

# With annual breakdown
cargo run --bin read_dividends -- --input ./dividends.divbin --ticker AAPL --annual
```

**Flags**:
- `--input`: Path to .divbin file
- `--ticker`: Show specific ticker only (optional)
- `--summary`: Show summary table instead of details
- `--annual`: Include annual breakdown for recent years

**Summary Output Example**:
```
Ticker  | Payments | Total Amount | Avg Dividend | Last Dividend
--------|----------|--------------|--------------|---------------
AAPL    |       87 |     $21.75   |     $0.2500  | $0.25
JNJ     |      104 |    $112.68   |     $1.0835  | $1.19
KO      |       96 |     $41.28   |     $0.4300  | $0.48
MSFT    |       73 |     $45.62   |     $0.6249  | $0.75
PG      |       91 |     $78.54   |     $0.8631  | $0.97
--------|----------|--------------|--------------|---------------
TOTAL   |      451 |    $299.87   |              |
```

### 9. Performance Characteristics

#### Extraction Performance
- **Database query**: O(n) where n = total dividends for requested tickers
- **Batch extraction**: Single query with `IN (...)` clause
- **Write speed**: ~500-1000 MB/s (SSD), limited by `BufWriter` flush

#### Binary File Operations

| Operation | Complexity | Notes |
|-----------|------------|-------|
| Write all tickers | O(n) | Single pass + index write |
| Read all tickers | O(n) | Sequential read of data + index |
| Read single ticker | O(m + k) | m=ticker count (index scan), k=dividends for ticker |
| File size | ~12 bytes/div | Header + 8 bytes/div + 24 bytes/ticker (index) |

**Example sizes**:
- 10 tickers, 1000 dividends total: ~12 KB
- 100 tickers, 10,000 dividends: ~122 KB
- 500 tickers, 50,000 dividends: ~612 KB

#### Optimization Opportunities

1. **Index as HashMap** (memory trade-off):
   ```rust
   // Instead of scanning index, build HashMap on first access
   lazy_static! {
       static ref TICKER_INDEX: HashMap<String, (u64, u32)> = {
           // Load index once, O(m) time, O(m) space
       };
   }

   // Then O(1) lookup instead of O(m) scan
   ```

2. **Memory-mapped files**:
   ```rust
   use memmap2::Mmap;

   let mmap = unsafe { Mmap::map(&file)? };
   // Zero-copy access to dividends
   // Trade-off: Holds file open, uses virtual memory
   ```

3. **Compressed archive**:
   ```rust
   // For long-term storage, compress the .divbin file
   // .divbin.zst (zstd compression): ~50-70% size reduction
   // Decompress on-demand or keep uncompressed in memory
   ```

### 10. Testing and Validation

#### Unit Tests

From `src/dividends.rs:381`:

**Binary Round-trip Test**:
```rust
#[test]
fn test_binary_roundtrip() {
    let mut test_data = HashMap::new();
    test_data.insert("AAPL".to_string(), vec![
        Dividend { date: NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), amount: 0.25 },
        Dividend { date: NaiveDate::from_ymd_opt(2024, 4, 15).unwrap(), amount: 0.26 },
    ]);

    save_dividends_to_binary(&test_data, &file_path).unwrap();
    let loaded = load_dividends_from_binary(&file_path).unwrap();

    assert_eq!(loaded.get("AAPL").unwrap()[0].amount, 0.25);
}
```

**Single Ticker Lookup Test**:
```rust
#[test]
fn test_single_ticker_load() {
    // Save multi-ticker file
    save_dividends_to_binary(&test_data, &file_path).unwrap();

    // Load single ticker
    let aapl = load_single_ticker_from_binary(&file_path, "AAPL").unwrap();
    assert!(aapl.is_some());

    // Non-existent ticker
    let fake = load_single_ticker_from_binary(&file_path, "FAKE").unwrap();
    assert!(fake.is_none());
}
```

#### Integration Testing

**Validate against known data**:
```rust
// Known: AAPL paid $0.25/quarter in 2024
let dividends = load_single_ticker_from_binary(path, "AAPL")?.unwrap();
let total_2024 = calculate_annual_dividend_yield(&dividends, 2024);
assert!((total_2024 - 1.00).abs() < 0.01); // ~$1.00/year
```

**Consistency checks**:
```rust
// All amounts should be positive
for div in &dividends {
    assert!(div.amount > 0.0);
}

// Dates should be in descending order (newest first)
for window in dividends.windows(2) {
    assert!(window[0].date >= window[1].date);
}
```

### 11. Common Patterns and Best Practices

#### Pattern 1: Lazy Loading

```rust
struct DividendCache {
    path: PathBuf,
    cache: HashMap<String, Vec<Dividend>>,
}

impl DividendCache {
    fn get(&mut self, ticker: &str) -> Result<&Vec<Dividend>> {
        if !self.cache.contains_key(ticker) {
            if let Some(divs) = load_single_ticker_from_binary(&self.path, ticker)? {
                self.cache.insert(ticker.to_string(), divs);
            }
        }
        self.cache.get(ticker).ok_or_else(|| anyhow::anyhow!("Ticker not found"))
    }
}
```

#### Pattern 2: Dividend Dates Mapping

```rust
// Build date → dividend map for fast date lookups
fn build_dividend_date_map(dividends: &[Dividend]) -> HashMap<NaiveDate, f64> {
    dividends.iter()
        .map(|d| (d.date, d.amount))
        .collect()
}

// Usage in backtest
let div_map = build_dividend_date_map(&dividends);
for date in date_range {
    if let Some(&dividend) = div_map.get(&date) {
        // Process dividend payment
        portfolio_cash += shares * dividend;
    }
}
```

#### Pattern 3: Dividend Growth Analysis

```rust
fn calculate_dividend_growth_rate(dividends: &[Dividend], years: usize) -> Option<f64> {
    if dividends.len() < years * 4 {
        return None; // Not enough data
    }

    let current_year = chrono::Local::now().year();
    let start_year = current_year - years as i32;

    let recent_annual = calculate_annual_dividend_yield(dividends, current_year);
    let past_annual = calculate_annual_dividend_yield(dividends, start_year);

    // CAGR formula
    let growth_rate = ((recent_annual / past_annual).powf(1.0 / years as f64) - 1.0) * 100.0;
    Some(growth_rate)
}

// Find dividend aristocrats (25+ years of growth)
let growth_rate = calculate_dividend_growth_rate(&dividends, 25)?;
if growth_rate > 0.0 {
    println!("Dividend aristocrat with {:.2}% CAGR", growth_rate);
}
```

### 12. Error Handling

#### Common Errors

**File Not Found**:
```rust
if !path.exists() {
    anyhow::bail!("Dividend file not found: {:?}", path);
}
```

**Invalid Magic Bytes**:
```rust
if header.magic != *MAGIC_BYTES {
    anyhow::bail!("Invalid file format: wrong magic bytes");
}
```

**Version Mismatch**:
```rust
if header.version != FORMAT_VERSION {
    anyhow::bail!("Unsupported format version: {}", header.version);
}
```

**Database Errors**:
```rust
let conn = Connection::open(db_path)
    .with_context(|| format!("Failed to open database at {:?}", db_path))?;
```

#### Defensive Programming

**Validate data ranges**:
```rust
// Dates should be reasonable
if dividend.date.year() < 1900 || dividend.date.year() > 2100 {
    eprintln!("Warning: Suspicious date: {}", dividend.date);
}

// Amounts should be positive and reasonable
if dividend.amount <= 0.0 || dividend.amount > 1000.0 {
    eprintln!("Warning: Suspicious amount: ${}", dividend.amount);
}
```

**Handle missing data gracefully**:
```rust
match load_single_ticker_from_binary(path, ticker)? {
    Some(dividends) => process_dividends(&dividends),
    None => {
        eprintln!("No dividend data for {}, using zero yield", ticker);
        Vec::new()
    }
}
```

## Summary

**Workflow**:
1. Extract dividends from SQLite → Binary file (.divbin)
2. Read entire file or single ticker as needed
3. Use helper functions for yield calculations and date filtering
4. Integrate into trading strategies and backtests

**Key Functions**:
- `extract_dividends_batch()` - Extract from database
- `save_dividends_to_binary()` - Write .divbin file
- `load_dividends_from_binary()` - Read all tickers
- `load_single_ticker_from_binary()` - Read single ticker (optimized)
- `calculate_total_dividends()` - Sum with date filtering
- `calculate_annual_dividend_yield()` - Annual dividend total

**Files**:
- `src/dividends.rs` - Core library
- `src/bin/extract_dividends.rs` - Extraction tool
- `src/bin/read_dividends.rs` - Reading/analysis tool
- `corporate_data_formats.md` - Binary format specification

**Performance**:
- Compact storage: ~12 bytes per dividend
- Fast single-ticker lookup: O(m + k) where m=tickers, k=dividends
- Production-ready for thousands of tickers and millions of dividend records
