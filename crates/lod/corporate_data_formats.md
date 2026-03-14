# Corporate Data File Formats

This document describes the file formats used for storing corporate action data (dividends and stock splits).

---

## DivBin File Format (.divbin)

### Overview
DivBin files are binary files that store dividend information for multiple stock tickers in a compact, efficient format with an index for fast lookups.

### File Structure

#### Header (18 bytes)
```
Offset  Size  Type    Description
------  ----  ------  -----------
0x00    4     [u8;4]  Magic bytes (0x44495644 = "DIVD" in ASCII)
0x04    2     u16     Format version (currently 1)
0x06    4     u32     Number of tickers in the file
0x0A    8     u64     File offset to index section
```

#### Data Section
Following the header, the data section contains ticker entries:

**Ticker Entry:**
```
Offset  Size  Type    Description
------  ----  ------  -----------
0x00    8     [u8;8]  Ticker symbol (padded with nulls)
0x08    4     u32     Number of dividend records
0x0C    var   records Dividend records array
```

**Dividend Record** (8 bytes each):
```
Offset  Size  Type    Description
------  ----  ------  -----------
0x00    4     u32     Date as days since Unix epoch (1970-01-01)
0x04    4     u32     Dividend amount in cents (to avoid float precision issues)
```

#### Index Section
At the end of the file (starting at index_offset from header):

**Index Entry** (24 bytes each):
```
Offset  Size  Type    Description
------  ----  ------  -----------
0x00    8     [u8;8]  Ticker symbol (padded with nulls)
0x08    8     u64     File offset to this ticker's data
0x10    4     u32     Number of dividend records
0x14    4     u32     Padding (for 8-byte alignment)
```

### Reading Example (Rust)

```rust
// Read header
let mut magic_bytes = [0u8; 4];
reader.read_exact(&mut magic_bytes)?;  // Should be "DIVD"
let version = reader.read_u16()?;      // Format version
let ticker_count = reader.read_u32()?; // Number of tickers
let index_offset = reader.read_u64()?; // Offset to index

// Jump to index section for efficient lookup
reader.seek(SeekFrom::Start(index_offset))?;

// Read index entries
for _ in 0..ticker_count {
    let mut ticker = [0u8; 8];
    reader.read_exact(&mut ticker)?;
    let file_offset = reader.read_u64()?;
    let record_count = reader.read_u32()?;
    let _padding = reader.read_u32()?;
    
    // Now can jump to file_offset to read this ticker's data
}

// To read a specific ticker's data:
reader.seek(SeekFrom::Start(file_offset))?;
let mut ticker_bytes = [0u8; 8];
reader.read_exact(&mut ticker_bytes)?;
let dividend_count = reader.read_u32()?;

// Read dividend records
for _ in 0..dividend_count {
    let date_days = reader.read_u32()?;     // Days since epoch
    let amount_cents = reader.read_u32()?;  // Amount in cents
    
    // Convert to usable values
    let date = epoch + Duration::days(date_days as i64);
    let amount = amount_cents as f64 / 100.0;
}
```

### Binary Layout Example

For a file with 2 tickers (AAPL with 2 dividends, MSFT with 1 dividend):

```
[Header: 18 bytes]
44 49 56 44           # Magic "DIVD"
01 00                 # Version 1
02 00 00 00           # 2 tickers
xx xx xx xx xx xx xx xx  # Index offset (8 bytes)

[Data Section]
[Ticker 1: AAPL at offset 18]
41 41 50 4C 00 00 00 00  # "AAPL" padded to 8 bytes
02 00 00 00              # 2 dividends

[AAPL Dividend 1]
xx xx xx xx  # Date days (4 bytes)
xx xx xx xx  # Amount cents (4 bytes)

[AAPL Dividend 2]
xx xx xx xx  # Date days (4 bytes)
xx xx xx xx  # Amount cents (4 bytes)

[Ticker 2: MSFT at offset 38]
4D 53 46 54 00 00 00 00  # "MSFT" padded to 8 bytes
01 00 00 00              # 1 dividend

[MSFT Dividend 1]
xx xx xx xx  # Date days (4 bytes)
xx xx xx xx  # Amount cents (4 bytes)

[Index Section at offset 54]
[Index Entry 1: AAPL]
41 41 50 4C 00 00 00 00  # "AAPL" padded
12 00 00 00 00 00 00 00  # File offset = 18
02 00 00 00              # 2 records
00 00 00 00              # Padding

[Index Entry 2: MSFT]
4D 53 46 54 00 00 00 00  # "MSFT" padded
26 00 00 00 00 00 00 00  # File offset = 38
01 00 00 00              # 1 record
00 00 00 00              # Padding
```

### Notes
- All multi-byte integers are stored in little-endian format
- Dates are stored as days since Unix epoch (1970-01-01) as u32
- Dividend amounts are stored as cents (u32) to avoid floating-point precision issues
- Ticker symbols are padded with null bytes to exactly 8 bytes
- The index section at the end allows for fast O(1) lookups of specific tickers
- The format is designed for both sequential reading and random access via the index
- No compression is applied to maintain simplicity and fast access

---

## Splits JSON File Format (.json)

### Overview
Splits JSON files store stock split information for multiple tickers in a human-readable JSON format. Each split record contains the date, ratio, and description of the split event.

### File Structure

#### Root Object
```json
{
  "tickers": {
    "TICKER1": [...],
    "TICKER2": [...],
    ...
  }
}
```

#### Split Record Structure
Each ticker maps to an array of split records:

```json
{
  "date": "YYYY-MM-DD",
  "ratio": float,
  "description": "string"
}
```

### Field Descriptions

#### Split Record Fields
- **date**: ISO 8601 date string (YYYY-MM-DD) when the split occurred
- **ratio**: Multiplication factor for shares (e.g., 2.0 for a 2:1 split)
- **description**: Human-readable description of the split (e.g., "2:1 split")

#### Split Ratio Interpretation
- `ratio > 1.0`: Forward split (increases share count)
  - Example: 2.0 means 2:1 split (1 share becomes 2 shares)
  - Example: 20.0 means 20:1 split (1 share becomes 20 shares)
- `ratio < 1.0`: Reverse split (decreases share count)
  - Example: 0.5 means 1:2 reverse split (2 shares become 1 share)
  - Example: 0.1 means 1:10 reverse split (10 shares become 1 share)

### Complete Example

```json
{
  "tickers": {
    "AAPL": [
      {
        "date": "2020-08-31",
        "ratio": 4.0,
        "description": "4:1 split"
      },
      {
        "date": "2014-06-09",
        "ratio": 7.0,
        "description": "7:1 split"
      }
    ],
    "TSLA": [
      {
        "date": "2022-08-25",
        "ratio": 3.0,
        "description": "3:1 split"
      },
      {
        "date": "2020-08-31",
        "ratio": 5.0,
        "description": "5:1 split"
      }
    ],
    "GE": [
      {
        "date": "2021-08-02",
        "ratio": 0.125,
        "description": "1:8 reverse split"
      }
    ],
    "MSFT": []
  }
}
```

### Reading Example (Python)

```python
import json
from datetime import datetime

def read_splits_file(filepath):
    with open(filepath, 'r') as f:
        data = json.load(f)
    
    splits_data = data['tickers']
    
    for ticker, splits in splits_data.items():
        print(f"\n{ticker}:")
        if not splits:
            print("  No splits")
        else:
            for split in splits:
                date = split['date']
                ratio = split['ratio']
                description = split['description']
                print(f"  {date}: {description} (ratio: {ratio}x)")

def calculate_adjusted_shares(ticker, splits, shares, as_of_date):
    """Calculate adjusted share count after applying splits"""
    adjusted_shares = shares
    
    for split in sorted(splits, key=lambda x: x['date'], reverse=True):
        split_date = datetime.strptime(split['date'], '%Y-%m-%d')
        if split_date <= as_of_date:
            adjusted_shares *= split['ratio']
    
    return adjusted_shares
```

### Reading Example (Rust)

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct SplitRecord {
    pub date: String,
    pub ratio: f64,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MultiTickerSplitsData {
    pub tickers: HashMap<String, Vec<SplitRecord>>,
}

fn read_splits_file(filepath: &str) -> Result<MultiTickerSplitsData, Box<dyn Error>> {
    let contents = std::fs::read_to_string(filepath)?;
    let data: MultiTickerSplitsData = serde_json::from_str(&contents)?;
    Ok(data)
}

fn apply_splits_to_price(
    splits: &[SplitRecord],
    price: f64,
    date: &str,
) -> f64 {
    let mut adjusted_price = price;
    
    // Apply splits that occurred after the given date
    for split in splits.iter() {
        if split.date > date {
            adjusted_price /= split.ratio;
        }
    }
    
    adjusted_price
}
```

### Usage Notes

1. **Chronological Order**: Splits are typically stored in chronological order (oldest to newest)
2. **Empty Arrays**: Tickers with no splits have empty arrays `[]`
3. **Price Adjustment**: To adjust historical prices, divide by the split ratio for splits occurring after the price date
5. **Cumulative Effect**: Multiple splits have a cumulative effect (multiply ratios together)

### Common Split Ratios

| Ratio | Description | Effect |
|-------|-------------|--------|
| 2.0 | 2:1 split | Doubles shares, halves price |
| 3.0 | 3:1 split | Triples shares, price ÷ 3 |
| 4.0 | 4:1 split | Quadruples shares, price ÷ 4 |
| 1.5 | 3:2 split | 1.5x shares, price ÷ 1.5 |
| 0.5 | 1:2 reverse | Halves shares, doubles price |
| 0.1 | 1:10 reverse | 1/10 shares, 10x price |

### Integration with Trading Systems

When using split data:
1. Always apply splits in chronological order
2. Consider the ex-date vs. record date distinction
3. Update position quantities and average costs accordingly
4. Ensure price charts reflect split-adjusted values for historical comparison

---

## Comparison of Formats

| Aspect | DivBin (.divbin) | Splits JSON (.json) |
|--------|------------------|---------------------|
| Format | Binary | Text (JSON) |
| Size | Compact (~16-20 bytes per record) | Larger (verbose JSON) |
| Read Speed | Very fast | Moderate |
| Human Readable | No | Yes |
| Editing | Requires specialized tools | Any text editor |
| Compression | Not needed (already compact) | Benefits from compression |
| Use Case | High-performance systems | Configuration, debugging |

---

## DivBin File Creation Code (Rust)

```rust
pub fn save_dividends_to_binary(dividends_map: &HashMap<String, Vec<Dividend>>, output_path: &Path) -> Result<()> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {:?}", parent))?;
    }

    let mut file = BufWriter::new(File::create(output_path)?);
    let mut index_entries = Vec::new();
    
    // Write header (will update index_offset later)
    let header = BinaryHeader {
        magic: *MAGIC_BYTES,
        version: FORMAT_VERSION,
        ticker_count: dividends_map.len() as u32,
        index_offset: 0, // Will be updated
    };
    
    file.write_all(unsafe {
        std::slice::from_raw_parts(&header as *const _ as *const u8, std::mem::size_of::<BinaryHeader>())
    })?;
    
    // Write data section
    for (ticker, dividends) in dividends_map {
        let offset = file.stream_position()?;
        
        // Create padded ticker
        let mut ticker_bytes = [0u8; 8];
        let ticker_slice = ticker.as_bytes();
        let len = ticker_slice.len().min(8);
        ticker_bytes[..len].copy_from_slice(&ticker_slice[..len]);
        
        // Write ticker header
        file.write_all(&ticker_bytes)?;
        file.write_all(&(dividends.len() as u32).to_le_bytes())?;
        
        // Write dividends
        for dividend in dividends {
            let binary_div = dividend.to_binary();
            file.write_all(unsafe {
                std::slice::from_raw_parts(&binary_div as *const _ as *const u8, std::mem::size_of::<BinaryDividend>())
            })?;
        }
        
        // Store index entry
        index_entries.push(IndexEntry {
            ticker: ticker_bytes,
            file_offset: offset,
            record_count: dividends.len() as u32,
            _padding: 0,
        });
    }
    
    // Remember where index starts
    let index_offset = file.stream_position()?;
    
    // Write index
    for entry in &index_entries {
        file.write_all(unsafe {
            std::slice::from_raw_parts(entry as *const _ as *const u8, std::mem::size_of::<IndexEntry>())
        })?;
    }
    
    // Go back and update header with index offset
    file.seek(SeekFrom::Start(0))?;
    let updated_header = BinaryHeader {
        magic: *MAGIC_BYTES,
        version: FORMAT_VERSION,
        ticker_count: dividends_map.len() as u32,
        index_offset,
    };
    file.write_all(unsafe {
        std::slice::from_raw_parts(&updated_header as *const _ as *const u8, std::mem::size_of::<BinaryHeader>())
    })?;
    
    file.flush()?;
    Ok(())
}

// Supporting structures and methods
const MAGIC_BYTES: &[u8; 4] = b"DIVD";
const FORMAT_VERSION: u16 = 1;

#[repr(C, packed)]
struct BinaryHeader {
    magic: [u8; 4],
    version: u16,
    ticker_count: u32,
    index_offset: u64,
}

#[repr(C, packed)]
struct BinaryDividend {
    date_days: u32,     // Days since Unix epoch
    amount_cents: u32,  // Amount in cents to avoid float precision issues
}

#[repr(C, packed)]
struct IndexEntry {
    ticker: [u8; 8],    // Padded ticker symbol
    file_offset: u64,   // Position in data section
    record_count: u32,  // Number of dividends
    _padding: u32,      // Align to 8 bytes
}

impl Dividend {
    fn to_binary(&self) -> BinaryDividend {
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let days = (self.date - epoch).num_days() as u32;
        let cents = (self.amount * 100.0).round() as u32;
        
        BinaryDividend {
            date_days: days,
            amount_cents: cents,
        }
    }
}
```