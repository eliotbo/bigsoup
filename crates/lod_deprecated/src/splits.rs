//! Stock split adjustment support for price data
//!
//! This module provides data structures and utilities for applying split adjustments
//! to historical price data. Prices are divided by the cumulative split ratio to
//! normalize them to the current share structure.
//!
//! # Example
//!
//! ```no_run
//! use lod::splits::{SplitAdjuster, load_splits_from_json};
//! use std::path::Path;
//!
//! let splits = load_splits_from_json(Path::new("splits.json"), "NVDA")?;
//! let adjuster = SplitAdjuster::new(splits);
//!
//! // Adjust a price from a specific timestamp
//! let adjusted_price = adjuster.adjust_price(raw_price, timestamp_ns);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Represents a single stock split event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockSplit {
    /// The date when the split occurred (YYYY-MM-DD)
    #[serde(with = "date_format")]
    pub date: NaiveDate,
    /// The split ratio as a multiplication factor (e.g., 10.0 for a 10:1 split)
    pub ratio: f64,
    /// Human-readable description (e.g., "10:1 split")
    pub description: String,
}

/// Serde module for NaiveDate serialization/deserialization
mod date_format {
    use chrono::NaiveDate;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(date: &NaiveDate, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&date.format("%Y-%m-%d").to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<NaiveDate, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(serde::de::Error::custom)
    }
}

/// Collection of splits for a single ticker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitCalendar {
    /// List of splits, should be sorted by date descending (newest first) for efficiency
    pub splits: Vec<StockSplit>,
}

/// Multi-ticker split data structure
#[derive(Debug, Serialize, Deserialize)]
pub struct MultiTickerSplitsData {
    /// Map from ticker symbol to list of splits
    pub tickers: HashMap<String, Vec<StockSplit>>,
}

/// Sentinel value for NULL prices in nanodollar format
pub const NULL_PRICE: i64 = i64::MAX;

/// Adjust a nanodollar i64 price by a split adjustment factor
///
/// This is the core scaling function that maintains nanodollar precision
/// while avoiding overflow. NULL prices (i64::MAX or -1) are preserved.
///
/// # Arguments
///
/// * `price` - Price in nanodollars (i64::MAX is treated as NULL)
/// * `factor` - The adjustment factor (typically from calculate_cumulative_adjustment)
///
/// # Returns
///
/// The adjusted price, or the original if it's NULL or factor is 1.0
///
/// # Example
///
/// ```
/// use lod::splits::adjust_price_by_factor;
///
/// let price = 1_000_000_000_000i64; // $1000.00 in nanodollars
/// let factor = 10.0; // 10:1 split
/// let adjusted = adjust_price_by_factor(price, factor);
/// assert_eq!(adjusted, 100_000_000_000); // $100.00
/// ```
pub fn adjust_price_by_factor(price: i64, factor: f64) -> i64 {
    if price == NULL_PRICE || price == -1 {
        return price;
    }

    if factor == 1.0 {
        return price;
    }

    // Divide price by factor, rounding to nearest
    // Use f64 for the calculation to maintain precision
    let adjusted = (price as f64) / factor;

    // Round to nearest i64, avoiding overflow
    if adjusted > (i64::MAX as f64) {
        i64::MAX
    } else if adjusted < (i64::MIN as f64) {
        i64::MIN
    } else {
        adjusted.round() as i64
    }
}

/// Efficiently computes split adjustment factors for timestamps
///
/// Caches adjustment factors by date to avoid redundant computation.
pub struct SplitAdjuster {
    splits: Vec<StockSplit>,
    /// Cache mapping date to cumulative adjustment factor
    cache: BTreeMap<NaiveDate, f64>,
}

impl SplitAdjuster {
    /// Create a new adjuster from a list of splits
    ///
    /// Splits are automatically sorted by date for correct calculation.
    pub fn new(splits: Vec<StockSplit>) -> Self {
        let mut splits = splits;
        // Sort by date ascending to ensure correct cumulative calculation
        splits.sort_by_key(|s| s.date);

        Self {
            splits,
            cache: BTreeMap::new(),
        }
    }

    /// Calculate the cumulative adjustment factor for a given date
    ///
    /// The factor is the product of all split ratios that occurred AFTER the given date.
    /// For dates after all splits, returns 1.0 (no adjustment needed).
    ///
    /// # Algorithm
    ///
    /// - For each split that occurred AFTER `as_of_date`, multiply the ratio
    /// - Splits on or before `as_of_date` are ignored (already incorporated)
    ///
    /// # Example
    ///
    /// Given splits: [2021-07-20: 4.0, 2024-06-10: 10.0]
    /// - Date 2020-01-01 → factor = 4.0 × 10.0 = 40.0 (before both)
    /// - Date 2022-01-01 → factor = 10.0 (after 2021, before 2024)
    /// - Date 2025-01-01 → factor = 1.0 (after both)
    pub fn calculate_cumulative_adjustment(&self, as_of_date: NaiveDate) -> f64 {
        let mut adjustment = 1.0;

        for split in &self.splits {
            if split.date > as_of_date {
                adjustment *= split.ratio;
            }
        }

        adjustment
    }

    /// Get the adjustment factor for a date, using cache if available
    pub fn factor_for_date(&mut self, date: NaiveDate) -> f64 {
        if let Some(&factor) = self.cache.get(&date) {
            return factor;
        }

        let factor = self.calculate_cumulative_adjustment(date);
        self.cache.insert(date, factor);
        factor
    }

    /// Get the adjustment factor for a timestamp in nanoseconds
    ///
    /// Converts the timestamp to a NaiveDate and looks up the adjustment factor.
    pub fn factor_for(&mut self, timestamp_ns: i64) -> f64 {
        use chrono::DateTime;

        // Convert nanosecond timestamp to DateTime<Utc>
        let dt = DateTime::from_timestamp(
            timestamp_ns / 1_000_000_000,
            (timestamp_ns % 1_000_000_000) as u32,
        )
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap());

        let date = dt.date_naive();
        self.factor_for_date(date)
    }

    /// Adjust an i64 price (in nanodollars) by the split factor for a given timestamp
    ///
    /// # Arguments
    ///
    /// * `price` - Price in nanodollars (i64::MAX is treated as NULL)
    /// * `timestamp_ns` - Timestamp in nanoseconds since epoch
    ///
    /// # Returns
    ///
    /// The split-adjusted price, or the original if it's NULL (i64::MAX) or factor is 1.0
    pub fn adjust_price(&mut self, price: i64, timestamp_ns: i64) -> i64 {
        let factor = self.factor_for(timestamp_ns);
        adjust_price_by_factor(price, factor)
    }

    /// Returns true if there are any splits configured
    pub fn has_splits(&self) -> bool {
        !self.splits.is_empty()
    }
}

/// Load splits for a specific ticker from a JSON file
///
/// # Arguments
///
/// * `path` - Path to the JSON file (expects MultiTickerSplitsData format)
/// * `ticker` - Ticker symbol to extract
///
/// # Returns
///
/// Vector of StockSplit for the requested ticker, or empty vector if not found
pub fn load_splits_from_json(
    path: &Path,
    ticker: &str,
) -> Result<Vec<StockSplit>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let data: MultiTickerSplitsData = serde_json::from_reader(reader)?;

    Ok(data
        .tickers
        .get(ticker)
        .cloned()
        .unwrap_or_default())
}

/// Load all splits from a JSON file
///
/// # Arguments
///
/// * `path` - Path to the JSON file
///
/// # Returns
///
/// MultiTickerSplitsData containing all tickers and their splits
pub fn load_all_splits_from_json(
    path: &Path,
) -> Result<MultiTickerSplitsData, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let data: MultiTickerSplitsData = serde_json::from_reader(reader)?;
    Ok(data)
}

/// Discover and load splits for a data file automatically
///
/// Looks for `splits.json` in the same directory as the data file.
/// If found, loads splits for the given ticker. If not found or ticker
/// not present, returns None.
///
/// # Arguments
///
/// * `data_file_path` - Path to the data file (.nohlcv, .ntrds, .nbbo)
/// * `ticker` - Ticker symbol to load splits for
/// * `warn_if_missing` - If true, prints warning when splits not found
///
/// # Returns
///
/// Some(SplitAdjuster) if splits were found and loaded, None otherwise
pub fn discover_and_load_splits(
    data_file_path: &Path,
    ticker: &str,
    warn_if_missing: bool,
) -> Option<SplitAdjuster> {
    // Get the directory containing the data file
    let dir = data_file_path.parent()?;

    // Look for splits.json in the same directory
    let splits_path = dir.join("splits.json");

    if !splits_path.exists() {
        if warn_if_missing {
            eprintln!(
                "⚠ Warning: No splits.json found in {} for {}",
                dir.display(),
                ticker
            );
        }
        return None;
    }

    // Try to load splits for this ticker
    match load_splits_from_json(&splits_path, ticker) {
        Ok(splits) if !splits.is_empty() => {
            eprintln!(
                "✓ Loaded {} split(s) for {} from {}",
                splits.len(),
                ticker,
                splits_path.display()
            );
            Some(SplitAdjuster::new(splits))
        }
        Ok(_) => {
            if warn_if_missing {
                eprintln!(
                    "⚠ Warning: {} not found in {}",
                    ticker,
                    splits_path.display()
                );
            }
            None
        }
        Err(e) => {
            eprintln!(
                "⚠ Warning: Failed to load splits from {}: {}",
                splits_path.display(),
                e
            );
            None
        }
    }
}

// ============================================================================
// Dividend Support
// ============================================================================

/// Type of dividend payment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DividendType {
    /// Regular cash dividend
    Cash = 0,
    /// Stock dividend
    Stock = 1,
    /// Special/one-time dividend
    Special = 2,
}

impl From<u8> for DividendType {
    fn from(value: u8) -> Self {
        match value {
            1 => DividendType::Stock,
            2 => DividendType::Special,
            _ => DividendType::Cash,
        }
    }
}

/// Represents a single dividend event
#[derive(Debug, Clone, PartialEq)]
pub struct DividendEvent {
    /// Ex-dividend date (date stock begins trading without dividend)
    pub ex_date: NaiveDate,
    /// Dividend amount per share (in dollars)
    pub amount: f64,
}

impl DividendEvent {
    /// Convert date in days since Unix epoch to NaiveDate
    fn days_to_date(days: u32) -> NaiveDate {
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        epoch + chrono::Duration::days(days as i64)
    }

    /// Create from raw binary data
    fn from_raw(date_days: u32, amount_cents: u32) -> Self {
        DividendEvent {
            ex_date: Self::days_to_date(date_days),
            amount: amount_cents as f64 / 100.0,
        }
    }
}

/// Load dividend events from a .divbin file for a specific ticker
///
/// The .divbin format supports multiple tickers in one file.
/// Format:
/// - Header: "DIVD" magic (4 bytes), version u16, ticker_count u32, index_offset u64
/// - Data sections: ticker (8 bytes), count (u32), records (date_days u32, amount_cents u32)
/// - Index: array of (ticker 8 bytes, offset u64, count u32, padding u32)
pub fn load_dividends_divbin(
    path: &Path,
    ticker: &str,
) -> Result<Vec<DividendEvent>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    // Read header (18 bytes)
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;

    if &magic != b"DIVD" {
        return Err(format!("Invalid magic bytes: expected DIVD, got {:?}", magic).into());
    }

    let version = read_u16(&mut reader)?;
    if version != 1 {
        return Err(format!("Unsupported version: {}", version).into());
    }

    let ticker_count = read_u32(&mut reader)?;
    let index_offset = read_u64(&mut reader)?;

    // Jump to index section
    reader.seek(SeekFrom::Start(index_offset))?;

    // Search for our ticker in the index
    for _ in 0..ticker_count {
        let mut ticker_bytes = [0u8; 8];
        reader.read_exact(&mut ticker_bytes)?;

        // Parse ticker name (null-terminated)
        let ticker_len = ticker_bytes.iter().position(|&b| b == 0).unwrap_or(8);
        let found_ticker = String::from_utf8(ticker_bytes[..ticker_len].to_vec())?;

        let file_offset = read_u64(&mut reader)?;
        let record_count = read_u32(&mut reader)?;
        let _padding = read_u32(&mut reader)?;

        if found_ticker == ticker {
            // Found our ticker, jump to its data
            reader.seek(SeekFrom::Start(file_offset))?;

            // Skip ticker bytes (8) and read count (4)
            reader.seek(SeekFrom::Current(8))?;
            let dividend_count = read_u32(&mut reader)?;

            if dividend_count != record_count {
                return Err(format!(
                    "Dividend count mismatch for {}: {} vs {}",
                    ticker, dividend_count, record_count
                )
                .into());
            }

            // Read dividend records (8 bytes each: date_days u32, amount_cents u32)
            let mut events = Vec::with_capacity(dividend_count as usize);
            for _ in 0..dividend_count {
                let date_days = read_u32(&mut reader)?;
                let amount_cents = read_u32(&mut reader)?;
                events.push(DividendEvent::from_raw(date_days, amount_cents));
            }

            return Ok(events);
        }
    }

    // Ticker not found
    Err(format!("Ticker {} not found in divbin file", ticker).into())
}

// Helper functions for reading binary data (little-endian)
fn read_u16(reader: &mut impl Read) -> std::io::Result<u16> {
    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_u32(reader: &mut impl Read) -> std::io::Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64(reader: &mut impl Read) -> std::io::Result<u64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

/// Discover and load dividend events for a data file automatically
///
/// Looks for `dividends.divbin` in the same directory as the data file.
/// If found, loads dividend events for the specified ticker.
/// If not found, returns empty vector.
///
/// # Arguments
///
/// * `data_file_path` - Path to the data file (.nohlcv, .ntrds, .nbbo)
/// * `ticker` - Ticker symbol to load dividends for
/// * `warn_if_missing` - If true, prints warning when dividends.divbin not found
///
/// # Returns
///
/// Vector of DividendEvent (empty if not found or on error)
pub fn discover_and_load_dividends(
    data_file_path: &Path,
    ticker: &str,
    warn_if_missing: bool,
) -> Vec<DividendEvent> {
    // Get the directory containing the data file
    let dir = match data_file_path.parent() {
        Some(d) => d,
        None => return Vec::new(),
    };

    // Look for dividends.divbin in the same directory
    let divbin_path = dir.join("dividends.divbin");

    if !divbin_path.exists() {
        if warn_if_missing {
            eprintln!(
                "⚠ Warning: No dividends.divbin found in {} for {}",
                dir.display(),
                ticker
            );
        }
        return Vec::new();
    }

    // Try to load dividends for this ticker
    match load_dividends_divbin(&divbin_path, ticker) {
        Ok(dividends) => {
            eprintln!(
                "✓ Loaded {} dividend(s) for {} from {}",
                dividends.len(),
                ticker,
                divbin_path.display()
            );
            dividends
        }
        Err(e) => {
            eprintln!(
                "⚠ Warning: Failed to load dividends from {}: {}",
                divbin_path.display(),
                e
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_split(date_str: &str, ratio: f64) -> StockSplit {
        StockSplit {
            date: NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap(),
            ratio,
            description: format!("{}:1 split", ratio),
        }
    }

    #[test]
    fn test_cumulative_adjustment_no_splits() {
        let adjuster = SplitAdjuster::new(vec![]);
        assert_eq!(adjuster.calculate_cumulative_adjustment(
            NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()
        ), 1.0);
    }

    #[test]
    fn test_cumulative_adjustment_single_split() {
        let splits = vec![make_test_split("2024-06-10", 10.0)];
        let adjuster = SplitAdjuster::new(splits);

        // Before split
        assert_eq!(
            adjuster.calculate_cumulative_adjustment(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
            10.0
        );

        // On split date - split already incorporated
        assert_eq!(
            adjuster.calculate_cumulative_adjustment(NaiveDate::from_ymd_opt(2024, 6, 10).unwrap()),
            1.0
        );

        // After split
        assert_eq!(
            adjuster.calculate_cumulative_adjustment(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()),
            1.0
        );
    }

    #[test]
    fn test_cumulative_adjustment_multiple_splits() {
        let splits = vec![
            make_test_split("2021-07-20", 4.0),
            make_test_split("2024-06-10", 10.0),
        ];
        let adjuster = SplitAdjuster::new(splits);

        // Before both splits
        assert_eq!(
            adjuster.calculate_cumulative_adjustment(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()),
            40.0
        );

        // After first split, before second
        assert_eq!(
            adjuster.calculate_cumulative_adjustment(NaiveDate::from_ymd_opt(2022, 1, 1).unwrap()),
            10.0
        );

        // After both splits
        assert_eq!(
            adjuster.calculate_cumulative_adjustment(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()),
            1.0
        );
    }

    #[test]
    fn test_adjust_price_basic() {
        let splits = vec![make_test_split("2024-06-10", 10.0)];
        let mut adjuster = SplitAdjuster::new(splits);

        // Use a timestamp from before the split (e.g., 2024-01-01 00:00:00 UTC)
        let ts_before = 1704067200_000_000_000i64; // 2024-01-01
        let price = 1_000_000_000_000i64; // $1000.00 in nanodollars

        let adjusted = adjuster.adjust_price(price, ts_before);
        assert_eq!(adjusted, 100_000_000_000); // $100.00 in nanodollars
    }

    #[test]
    fn test_adjust_price_null() {
        let splits = vec![make_test_split("2024-06-10", 10.0)];
        let mut adjuster = SplitAdjuster::new(splits);

        let ts = 1704067200_000_000_000i64;
        assert_eq!(adjuster.adjust_price(i64::MAX, ts), i64::MAX);
        assert_eq!(adjuster.adjust_price(-1, ts), -1);
    }

    #[test]
    fn test_adjust_price_no_splits() {
        let mut adjuster = SplitAdjuster::new(vec![]);

        let ts = 1704067200_000_000_000i64;
        let price = 1_000_000_000_000i64;
        assert_eq!(adjuster.adjust_price(price, ts), price);
    }

    #[test]
    fn test_reverse_split() {
        // 1:2 reverse split (0.5 ratio) means price should be multiplied by 2
        let splits = vec![make_test_split("2024-06-10", 0.5)];
        let mut adjuster = SplitAdjuster::new(splits);

        let ts_before = 1704067200_000_000_000i64; // 2024-01-01
        let price = 100_000_000_000i64; // $100.00

        let adjusted = adjuster.adjust_price(price, ts_before);
        assert_eq!(adjusted, 200_000_000_000); // $200.00 (doubled)
    }
}
