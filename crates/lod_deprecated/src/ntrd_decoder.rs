/// Standalone library for decoding .ntrd binary files
///
/// This module provides a complete decoder for the custom .ntrd (trades) format.
/// It can be copied as-is to another crate and used independently.
///
/// # Usage Example
/// ```rust,no_run
/// use lod::ntrd_decoder::{NtrdReader, TradeRecord};
///
/// fn read_trade_data(path: &str) -> Result<Vec<TradeRecord>, Box<dyn std::error::Error>> {
///     let mut reader = NtrdReader::open(path)?;
///     let records = reader.read_records(0, 100)?;
///     Ok(records)
/// }
/// ```
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Magic bytes for NTRD files
pub const NTRD_MAGIC: [u8; 4] = *b"NTRD";

/// Expected version
pub const EXPECTED_VERSION: u16 = 1;

/// Size of a single trade record in bytes
const RECORD_SIZE: usize = 64;

/// Custom error type for NTRD operations
#[derive(Debug)]
pub enum NtrdError {
    Io(io::Error),
    InvalidMagic([u8; 4]),
    InvalidVersion(u16),
    InvalidUtf8(std::string::FromUtf8Error),
    InvalidData(String),
}

impl std::fmt::Display for NtrdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NtrdError::Io(e) => write!(f, "IO error: {}", e),
            NtrdError::InvalidMagic(magic) => write!(f, "Invalid magic bytes: {:?}", magic),
            NtrdError::InvalidVersion(v) => write!(f, "Invalid version: {}", v),
            NtrdError::InvalidUtf8(e) => write!(f, "Invalid UTF-8: {}", e),
            NtrdError::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
        }
    }
}

impl std::error::Error for NtrdError {}

impl From<io::Error> for NtrdError {
    fn from(e: io::Error) -> Self {
        NtrdError::Io(e)
    }
}

impl From<std::string::FromUtf8Error> for NtrdError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        NtrdError::InvalidUtf8(e)
    }
}

pub type Result<T> = std::result::Result<T, NtrdError>;

/// Header structure for .ntrd files
#[derive(Debug, Clone)]
pub struct NtrdHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub header_length: u16,
    pub created_at_ns: u64,
    pub record_count: u64,
    pub instrument_id: u32,
    pub symbol: String,
    pub footprint_flags: u8,
    pub header_checksum: u32,
    pub metadata: BTreeMap<String, String>,
}

/// Single trade record (64 bytes)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TradeRecord {
    /// Event timestamp in nanoseconds since Unix epoch
    pub ts_event: u64,
    /// Receive timestamp in nanoseconds
    pub ts_recv: u64,
    /// Price in nanodollars (divide by 1e9 for dollars)
    pub price: i64,
    /// Trade size in shares
    pub size: u32,
    /// Trade side as ASCII char ('B'=buy, 'S'=sell, 'A'=ask, etc)
    pub side: u8,
    /// Trade condition flags
    pub flags: u8,
    /// Exchange/publisher ID
    pub exchange: u16,
    /// Unique trade sequence number
    pub trade_id: u64,
}

impl TradeRecord {
    /// Create an adjusted copy of this record using a split adjustment factor
    ///
    /// Adjusts the price by dividing by the factor, preserving NULL values.
    /// Size and other fields remain unchanged.
    pub fn with_split_adjustment(&self, factor: f64) -> Self {
        use crate::splits::adjust_price_by_factor;

        if factor == 1.0 {
            return *self;
        }

        TradeRecord {
            ts_event: self.ts_event,
            ts_recv: self.ts_recv,
            price: adjust_price_by_factor(self.price, factor),
            size: self.size,
            side: self.side,
            flags: self.flags,
            exchange: self.exchange,
            trade_id: self.trade_id,
        }
    }

    /// Decode a record from a 64-byte buffer
    pub fn from_bytes(buffer: &[u8; 64]) -> Self {
        let ts_event = u64::from_le_bytes(buffer[0..8].try_into().unwrap());
        let ts_recv = u64::from_le_bytes(buffer[8..16].try_into().unwrap());
        let price = i64::from_le_bytes(buffer[16..24].try_into().unwrap());
        let size = u32::from_le_bytes(buffer[24..28].try_into().unwrap());
        let side = buffer[28];
        let flags = buffer[29];
        let exchange = u16::from_le_bytes(buffer[30..32].try_into().unwrap());
        let trade_id = u64::from_le_bytes(buffer[32..40].try_into().unwrap());
        // bytes 40..64 are reserved/padding

        TradeRecord {
            ts_event,
            ts_recv,
            price,
            size,
            side,
            flags,
            exchange,
            trade_id,
        }
    }

    /// Encode the record to a 64-byte buffer
    pub fn to_bytes(&self) -> [u8; 64] {
        let mut buffer = [0u8; 64];

        buffer[0..8].copy_from_slice(&self.ts_event.to_le_bytes());
        buffer[8..16].copy_from_slice(&self.ts_recv.to_le_bytes());
        buffer[16..24].copy_from_slice(&self.price.to_le_bytes());
        buffer[24..28].copy_from_slice(&self.size.to_le_bytes());
        buffer[28] = self.side;
        buffer[29] = self.flags;
        buffer[30..32].copy_from_slice(&self.exchange.to_le_bytes());
        buffer[32..40].copy_from_slice(&self.trade_id.to_le_bytes());
        // bytes 40..64 remain zero (reserved)

        buffer
    }

    /// Convert timestamp to seconds since Unix epoch
    pub fn timestamp_secs(&self) -> f64 {
        self.ts_event as f64 / 1_000_000_000.0
    }

    /// Convert price from nanodollars to dollars
    pub fn price_as_float(&self) -> f64 {
        self.price as f64 / 1_000_000_000.0
    }

    /// Get the side as a char
    pub fn side_char(&self) -> char {
        self.side as char
    }

    /// Get the side as a readable string
    pub fn side_str(&self) -> &str {
        match self.side as char {
            'B' => "Buy",
            'S' => "Sell",
            'A' => "Sell", // 'A' indicates aggressor hit the bid (sell-side)
            'N' => "Unknown",
            _ => "Unknown",
        }
    }

    /// Check if this is an odd lot trade (less than 100 shares typically)
    pub fn is_odd_lot(&self) -> bool {
        self.flags & 0x80 != 0
    }

    /// Check if this is an opening trade
    pub fn is_opening(&self) -> bool {
        self.flags & 0x10 != 0
    }

    /// Check if this is a closing trade
    pub fn is_closing(&self) -> bool {
        self.flags & 0x20 != 0
    }

    /// Check if this is a buy trade
    pub fn is_buy(&self) -> bool {
        self.side as char == 'B'
    }

    /// Check if this is a sell trade
    pub fn is_sell(&self) -> bool {
        let side_char = self.side as char;
        side_char == 'S' || side_char == 'A' // Both 'S' and 'A' indicate sell-side
    }

    /// Check if aggressor side is known
    pub fn has_known_side(&self) -> bool {
        let side_char = self.side as char;
        side_char == 'B' || side_char == 'S' || side_char == 'A'
    }

    /// Get aggressor side description
    pub fn aggressor_side(&self) -> &str {
        match self.side as char {
            'B' => "Buyer (lifted ask)",
            'S' | 'A' => "Seller (hit bid)",
            _ => "Unknown",
        }
    }

    /// Calculate trade value (price * size)
    pub fn value(&self) -> f64 {
        self.price_as_float() * self.size as f64
    }

    /// Convert to PlotTrade for visualization
    pub fn to_plot_trade(&self) -> crate::levels::PlotTrade {
        crate::levels::PlotTrade::new(
            self.ts_event as i64,
            self.price_as_float() as f32,
            self.size as f32,
            self.side,
            self.flags,
            self.exchange,
        )
    }
}

/// Reader for .ntrd files
pub struct NtrdReader {
    file: File,
    header: NtrdHeader,
    payload_offset: u64,
    /// Optional split adjuster for price adjustments
    split_adjuster: Option<crate::splits::SplitAdjuster>,
}

impl NtrdReader {
    /// Open and parse a .ntrd file
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_internal(path, false)
    }

    /// Open and parse a .ntrd file with automatic split discovery
    ///
    /// Automatically looks for `splits.json` in the same directory and loads
    /// splits for the symbol in the file. Warns if splits are not found.
    pub fn open_with_auto_splits(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_internal(path, true)
    }

    /// Internal open implementation
    fn open_internal(path: impl AsRef<Path>, auto_discover_splits: bool) -> Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let mut file = File::open(&path_buf)?;
        let (header, payload_offset) = Self::read_header(&mut file)?;

        // Auto-discover splits if requested
        let split_adjuster = if auto_discover_splits {
            crate::splits::discover_and_load_splits(&path_buf, &header.symbol, true)
        } else {
            None
        };

        Ok(NtrdReader {
            file,
            header,
            payload_offset,
            split_adjuster,
        })
    }

    /// Set a split adjuster for this reader
    ///
    /// When set, all prices returned by this reader will be adjusted using
    /// the provided split adjuster.
    pub fn with_split_adjuster(mut self, adjuster: crate::splits::SplitAdjuster) -> Self {
        self.split_adjuster = Some(adjuster);
        self
    }

    /// Set a split adjuster by reference (useful after construction)
    pub fn set_split_adjuster(&mut self, adjuster: Option<crate::splits::SplitAdjuster>) {
        self.split_adjuster = adjuster;
    }

    /// Get the header
    pub fn header(&self) -> &NtrdHeader {
        &self.header
    }

    /// Get the total number of records
    pub fn record_count(&self) -> u64 {
        self.header.record_count
    }

    /// Get the symbol
    pub fn symbol(&self) -> &str {
        &self.header.symbol
    }

    /// Read a single record by index
    pub fn read_record(&mut self, index: u64) -> Result<TradeRecord> {
        if index >= self.header.record_count {
            return Err(NtrdError::InvalidData(format!(
                "Index {} out of bounds (max: {})",
                index,
                self.header.record_count - 1
            )));
        }

        let offset = self.payload_offset + (index * RECORD_SIZE as u64);
        self.file.seek(SeekFrom::Start(offset))?;

        let mut buffer = [0u8; 64];
        self.file.read_exact(&mut buffer)?;

        let mut record = TradeRecord::from_bytes(&buffer);

        // Apply split adjustment if configured
        if let Some(adjuster) = &mut self.split_adjuster {
            let factor = adjuster.factor_for(record.ts_event as i64);
            record = record.with_split_adjustment(factor);
        }

        Ok(record)
    }

    /// Read multiple records starting from an index
    pub fn read_records(&mut self, start: u64, count: usize) -> Result<Vec<TradeRecord>> {
        if start >= self.header.record_count {
            return Err(NtrdError::InvalidData(format!(
                "Start index {} out of bounds",
                start
            )));
        }

        let available = (self.header.record_count - start) as usize;
        let to_read = count.min(available);

        let mut records = Vec::with_capacity(to_read);
        let offset = self.payload_offset + (start * RECORD_SIZE as u64);
        self.file.seek(SeekFrom::Start(offset))?;

        for _ in 0..to_read {
            let mut buffer = [0u8; 64];
            self.file.read_exact(&mut buffer)?;
            let mut record = TradeRecord::from_bytes(&buffer);

            // Apply split adjustment if configured
            if let Some(adjuster) = &mut self.split_adjuster {
                let factor = adjuster.factor_for(record.ts_event as i64);
                record = record.with_split_adjustment(factor);
            }

            records.push(record);
        }

        Ok(records)
    }

    /// Read all records (use with caution for large files)
    pub fn read_all(&mut self) -> Result<Vec<TradeRecord>> {
        self.read_records(0, self.header.record_count as usize)
    }

    /// Filter records by criteria
    pub fn read_filtered<F>(
        &mut self,
        start: u64,
        count: usize,
        predicate: F,
    ) -> Result<Vec<TradeRecord>>
    where
        F: Fn(&TradeRecord) -> bool,
    {
        let records = self.read_records(start, count)?;
        Ok(records.into_iter().filter(predicate).collect())
    }

    /// Read only buy trades
    pub fn read_buys(&mut self, start: u64, count: usize) -> Result<Vec<TradeRecord>> {
        self.read_filtered(start, count, |r| r.is_buy())
    }

    /// Read only sell trades
    pub fn read_sells(&mut self, start: u64, count: usize) -> Result<Vec<TradeRecord>> {
        self.read_filtered(start, count, |r| r.is_sell())
    }

    /// Read only trades above a certain size
    pub fn read_above_size(
        &mut self,
        start: u64,
        count: usize,
        min_size: u32,
    ) -> Result<Vec<TradeRecord>> {
        self.read_filtered(start, count, |r| r.size >= min_size)
    }

    /// Read header from a file
    fn read_header(file: &mut File) -> Result<(NtrdHeader, u64)> {
        // Read fixed prefix (8 bytes)
        let mut prefix = [0u8; 8];
        file.read_exact(&mut prefix)?;

        let magic = [prefix[0], prefix[1], prefix[2], prefix[3]];
        if magic != NTRD_MAGIC {
            return Err(NtrdError::InvalidMagic(magic));
        }

        let version = u16::from_le_bytes([prefix[4], prefix[5]]);
        // Allow different versions but warn
        if version != EXPECTED_VERSION {
            eprintln!(
                "Warning: Version {} (expected {})",
                version, EXPECTED_VERSION
            );
        }

        let header_length = u16::from_le_bytes([prefix[6], prefix[7]]) as usize;

        // Read full header
        file.seek(SeekFrom::Start(0))?;
        let mut header_bytes = vec![0u8; header_length];
        file.read_exact(&mut header_bytes)?;

        // Parse header fields
        let mut offset = 8;

        let created_at_ns =
            u64::from_le_bytes(header_bytes[offset..offset + 8].try_into().unwrap());
        offset += 8;

        let record_count = u64::from_le_bytes(header_bytes[offset..offset + 8].try_into().unwrap());
        offset += 8;

        let instrument_id =
            u32::from_le_bytes(header_bytes[offset..offset + 4].try_into().unwrap());
        offset += 4;

        let symbol_len = header_bytes[offset] as usize;
        offset += 1;

        let symbol = String::from_utf8(header_bytes[offset..offset + symbol_len].to_vec())?;
        offset += symbol_len;

        let footprint_flags = header_bytes[offset];
        offset += 1;

        let header_checksum =
            u32::from_le_bytes(header_bytes[offset..offset + 4].try_into().unwrap());
        offset += 4;

        // Parse metadata if present
        let metadata = if offset < header_length {
            Self::decode_metadata(&header_bytes[offset..])?
        } else {
            BTreeMap::new()
        };

        let header = NtrdHeader {
            magic,
            version,
            header_length: header_length as u16,
            created_at_ns,
            record_count,
            instrument_id,
            symbol,
            footprint_flags,
            header_checksum,
            metadata,
        };

        Ok((header, header_length as u64))
    }

    /// Decode metadata from bytes
    fn decode_metadata(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
        let mut metadata = BTreeMap::new();
        let mut offset = 0;

        while offset < bytes.len() {
            // Check for end or insufficient bytes
            if offset >= bytes.len() {
                break;
            }

            let key_len = bytes[offset] as usize;
            offset += 1;

            if key_len == 0 || offset + key_len > bytes.len() {
                break;
            }

            let key = String::from_utf8(bytes[offset..offset + key_len].to_vec())?;
            offset += key_len;

            if offset + 2 > bytes.len() {
                break;
            }

            let value_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;

            if offset + value_len > bytes.len() {
                break;
            }

            let value = String::from_utf8(bytes[offset..offset + value_len].to_vec())?;
            offset += value_len;

            metadata.insert(key, value);
        }

        Ok(metadata)
    }
}

/// Iterator for reading records sequentially
pub struct RecordIterator<'a> {
    reader: &'a mut NtrdReader,
    current: u64,
}

impl<'a> RecordIterator<'a> {
    pub fn new(reader: &'a mut NtrdReader) -> Self {
        RecordIterator { reader, current: 0 }
    }
}

impl<'a> Iterator for RecordIterator<'a> {
    type Item = Result<TradeRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.reader.record_count() {
            return None;
        }

        let result = self.reader.read_record(self.current);
        self.current += 1;
        Some(result)
    }
}

/// Statistics calculator for trade data
#[derive(Debug, Default)]
pub struct TradeStats {
    pub record_count: usize,
    pub buy_count: usize,
    pub sell_count: usize,
    pub other_count: usize,
    pub odd_lot_count: usize,
    pub opening_count: usize,
    pub closing_count: usize,
    pub min_price: Option<f64>,
    pub max_price: Option<f64>,
    pub avg_price: Option<f64>,
    pub min_size: Option<u32>,
    pub max_size: Option<u32>,
    pub avg_size: Option<f64>,
    pub total_volume: u64,
    pub total_value: f64,
    pub vwap: Option<f64>,
}

impl TradeStats {
    pub fn calculate(records: &[TradeRecord]) -> Self {
        let mut stats = Self {
            record_count: records.len(),
            ..Default::default()
        };

        if records.is_empty() {
            return stats;
        }

        let mut price_sum = 0.0;
        let mut size_sum = 0u64;
        let mut value_sum = 0.0;

        for record in records {
            let price = record.price_as_float();
            let size = record.size;
            let value = record.value();

            // Categorize trades
            if record.is_buy() {
                stats.buy_count += 1;
            } else if record.is_sell() {
                stats.sell_count += 1;
            } else {
                stats.other_count += 1;
            }

            // Special trade types
            if record.is_odd_lot() {
                stats.odd_lot_count += 1;
            }
            if record.is_opening() {
                stats.opening_count += 1;
            }
            if record.is_closing() {
                stats.closing_count += 1;
            }

            // Price statistics
            stats.min_price = Some(stats.min_price.map_or(price, |min| min.min(price)));
            stats.max_price = Some(stats.max_price.map_or(price, |max| max.max(price)));
            price_sum += price;

            // Size statistics
            stats.min_size = Some(stats.min_size.map_or(size, |min| min.min(size)));
            stats.max_size = Some(stats.max_size.map_or(size, |max| max.max(size)));
            size_sum += size as u64;

            // Value statistics
            value_sum += value;
        }

        stats.total_volume = size_sum;
        stats.total_value = value_sum;

        if !records.is_empty() {
            stats.avg_price = Some(price_sum / records.len() as f64);
            stats.avg_size = Some(size_sum as f64 / records.len() as f64);
        }

        if size_sum > 0 {
            stats.vwap = Some(value_sum / size_sum as f64);
        }

        stats
    }

    /// Calculate time-weighted statistics
    pub fn calculate_time_weighted(records: &[TradeRecord]) -> TimeWeightedStats {
        if records.is_empty() {
            return TimeWeightedStats::default();
        }

        let mut stats = TimeWeightedStats {
            start_time: records[0].ts_event,
            end_time: records[records.len() - 1].ts_event,
            ..Default::default()
        };

        // Calculate trades per second
        let duration_ns = stats.end_time - stats.start_time;
        if duration_ns > 0 {
            let duration_secs = duration_ns as f64 / 1_000_000_000.0;
            stats.trades_per_second = records.len() as f64 / duration_secs;

            let total_volume: u64 = records.iter().map(|r| r.size as u64).sum();
            stats.volume_per_second = total_volume as f64 / duration_secs;
        }

        stats
    }
}

/// Time-weighted statistics
#[derive(Debug, Default)]
pub struct TimeWeightedStats {
    pub start_time: u64,
    pub end_time: u64,
    pub trades_per_second: f64,
    pub volume_per_second: f64,
}

/// Implement QuoteLike trait for aggregation
impl crate::traits::QuoteLike for TradeRecord {
    fn timestamp(&self) -> i64 {
        self.ts_event as i64
    }

    fn open(&self) -> f64 {
        // For a single trade, all OHLC values are the same price
        self.price_as_float()
    }

    fn high(&self) -> f64 {
        self.price_as_float()
    }

    fn low(&self) -> f64 {
        self.price_as_float()
    }

    fn close(&self) -> f64 {
        self.price_as_float()
    }

    fn volume(&self) -> f64 {
        self.size as f64
    }

    fn bid(&self) -> Option<f64> {
        // If this was a sell (hit bid), the trade price approximates the bid
        if self.is_sell() && self.has_known_side() {
            Some(self.price_as_float())
        } else {
            None
        }
    }

    fn ask(&self) -> Option<f64> {
        // If this was a buy (lifted ask), the trade price approximates the ask
        if self.is_buy() {
            Some(self.price_as_float())
        } else {
            None
        }
    }

    fn count(&self) -> Option<u32> {
        // A single trade has count of 1
        Some(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trade_record_round_trip() {
        let record = TradeRecord {
            ts_event: 1234567890000000000,
            ts_recv: 1234567890000000001,
            price: 25500000000, // $25.50
            size: 100,
            side: b'B',
            flags: 0x00,
            exchange: 1,
            trade_id: 12345,
        };

        let bytes = record.to_bytes();
        let decoded = TradeRecord::from_bytes(&bytes);

        assert_eq!(record, decoded);
    }

    #[test]
    fn test_price_conversion() {
        let record = TradeRecord {
            ts_event: 0,
            ts_recv: 0,
            price: 25500000000, // $25.50
            size: 100,
            side: b'B',
            flags: 0x00,
            exchange: 1,
            trade_id: 0,
        };

        let price = record.price_as_float();
        assert!((price - 25.50).abs() < 0.0001);
    }

    #[test]
    fn test_trade_value() {
        let record = TradeRecord {
            ts_event: 0,
            ts_recv: 0,
            price: 25500000000, // $25.50
            size: 100,
            side: b'B',
            flags: 0x00,
            exchange: 1,
            trade_id: 0,
        };

        let value = record.value();
        assert!((value - 2550.0).abs() < 0.01); // 100 shares * $25.50
    }

    #[test]
    fn test_side_detection() {
        let buy_record = TradeRecord {
            ts_event: 0,
            ts_recv: 0,
            price: 0,
            size: 0,
            side: b'B',
            flags: 0x00,
            exchange: 0,
            trade_id: 0,
        };

        assert!(buy_record.is_buy());
        assert!(!buy_record.is_sell());
        assert_eq!(buy_record.side_str(), "Buy");

        let sell_record = TradeRecord {
            side: b'S',
            ..buy_record
        };

        assert!(!sell_record.is_buy());
        assert!(sell_record.is_sell());
        assert_eq!(sell_record.side_str(), "Sell");
    }

    #[test]
    fn test_quote_like_trait() {
        use crate::traits::QuoteLike;

        let buy_trade = TradeRecord {
            ts_event: 1234567890000000000,
            ts_recv: 0,
            price: 25500000000, // $25.50
            size: 100,
            side: b'B',
            flags: 0x00,
            exchange: 0,
            trade_id: 0,
        };

        // Test QuoteLike implementation
        assert_eq!(buy_trade.timestamp(), 1234567890000000000);
        assert!((buy_trade.open() - 25.50).abs() < 0.0001);
        assert!((buy_trade.high() - 25.50).abs() < 0.0001);
        assert!((buy_trade.low() - 25.50).abs() < 0.0001);
        assert!((buy_trade.close() - 25.50).abs() < 0.0001);
        assert_eq!(buy_trade.volume(), 100.0);
        assert_eq!(buy_trade.count(), Some(1));

        // Buy trade should have ask price (lifted ask)
        assert_eq!(buy_trade.ask(), Some(25.50));
        assert_eq!(buy_trade.bid(), None);

        let sell_trade = TradeRecord {
            side: b'A', // 'A' indicates sell-side aggressor
            ..buy_trade
        };

        // Sell trade should have bid price (hit bid)
        assert_eq!(sell_trade.bid(), Some(25.50));
        assert_eq!(sell_trade.ask(), None);
    }

    #[test]
    fn test_flag_detection() {
        let odd_lot = TradeRecord {
            ts_event: 0,
            ts_recv: 0,
            price: 0,
            size: 50,
            side: b'B',
            flags: 0x80,
            exchange: 0,
            trade_id: 0,
        };

        assert!(odd_lot.is_odd_lot());
        assert!(!odd_lot.is_opening());
        assert!(!odd_lot.is_closing());

        let opening = TradeRecord {
            flags: 0x10,
            ..odd_lot
        };

        assert!(!opening.is_odd_lot());
        assert!(opening.is_opening());
        assert!(!opening.is_closing());

        let closing = TradeRecord {
            flags: 0x20,
            ..odd_lot
        };

        assert!(!closing.is_odd_lot());
        assert!(!closing.is_opening());
        assert!(closing.is_closing());
    }

    #[test]
    fn test_cien_data_for_f64_max_values() {
        let path = "/workspace/workspace/bb/beta_breaker/data/consolidated/hedgy-test/CIEN/2018-05-01_to_2025-09-18-trades.ntrd";

        // Skip test if file doesn't exist (e.g., in CI environments)
        if !std::path::Path::new(path).exists() {
            println!("Skipping test: data file not found at {}", path);
            return;
        }

        let mut reader = match NtrdReader::open(path) {
            Ok(r) => r,
            Err(e) => {
                panic!("Failed to open CIEN test data: {}", e);
            }
        };

        println!("Testing CIEN data file: {}", path);
        println!("Symbol: {}", reader.symbol());
        println!("Total records: {}", reader.record_count());

        let mut f64_max_count = 0;
        let mut null_price_count = 0;
        let mut total_checked = 0;
        let batch_size = 10000;

        // Read data in batches to manage memory
        let mut start = 0;
        while start < reader.record_count() {
            let count = std::cmp::min(batch_size, (reader.record_count() - start) as usize);

            let records = match reader.read_records(start, count) {
                Ok(records) => records,
                Err(e) => {
                    panic!("Failed to read records starting at {}: {}", start, e);
                }
            };

            for (i, record) in records.iter().enumerate() {
                total_checked += 1;

                let price_float = record.price_as_float();

                // Check if price equals f64::MAX (indicates null/invalid data)
                if price_float == f64::MAX {
                    f64_max_count += 1;
                    println!(
                        "Found f64::MAX price at record {}: {:?}",
                        start + i as u64,
                        record
                    );
                }

                // Check if the original price field is at extreme values that might indicate null
                if record.price == i64::MAX || record.price == i64::MIN {
                    null_price_count += 1;
                    println!(
                        "Found extreme i64 price at record {}: price={}, as_float={}",
                        start + i as u64,
                        record.price,
                        price_float
                    );
                }
            }

            start += count as u64;

            // Print progress every million records
            if total_checked % 1_000_000 == 0 {
                println!("Checked {} records so far...", total_checked);
            }
        }

        println!("Test Results:");
        println!("Total records checked: {}", total_checked);
        println!("Records with f64::MAX price: {}", f64_max_count);
        println!(
            "Records with extreme i64 price values: {}",
            null_price_count
        );

        if f64_max_count > 0 {
            println!(
                "WARNING: Found {} records with f64::MAX values (potential nulls)",
                f64_max_count
            );
        }

        if null_price_count > 0 {
            println!(
                "WARNING: Found {} records with extreme i64 price values",
                null_price_count
            );
        }

        // The test passes regardless of whether null values are found - we're just reporting
        println!("CIEN data analysis complete. Check output above for null value detection.");
    }

    #[test]
    fn investigate_extreme_value_at_record_16797686() {
        let path = "/workspace/workspace/bb/beta_breaker/data/consolidated/hedgy-test/CIEN/2018-05-01_to_2025-09-18-trades.ntrd";

        if !std::path::Path::new(path).exists() {
            println!("Skipping test: data file not found at {}", path);
            return;
        }

        let mut reader = match NtrdReader::open(path) {
            Ok(r) => r,
            Err(e) => {
                panic!("Failed to open CIEN test data: {}", e);
            }
        };

        println!("=== INVESTIGATING EXTREME VALUE AT RECORD 16797686 ===");
        println!("File: {}", path);
        println!("Symbol: {}", reader.symbol());
        println!("Total records: {}", reader.record_count());

        let target_record = 16797686u64;
        let context_range = 50; // Look at 50 records before and after

        // Calculate the range to examine
        let start_idx = if target_record >= context_range {
            target_record - context_range
        } else {
            0
        };
        let end_idx = std::cmp::min(target_record + context_range + 1, reader.record_count());
        let count = (end_idx - start_idx) as usize;

        println!(
            "\nExamining records {} to {} (context around target)",
            start_idx,
            end_idx - 1
        );

        let records = match reader.read_records(start_idx, count) {
            Ok(records) => records,
            Err(e) => {
                panic!("Failed to read records: {}", e);
            }
        };

        println!("\n=== DETAILED RECORD ANALYSIS ===");
        for (i, record) in records.iter().enumerate() {
            let actual_index = start_idx + i as u64;
            let is_target = actual_index == target_record;
            let prefix = if is_target {
                ">>> TARGET >>>"
            } else {
                "           "
            };

            let price_float = record.price_as_float();
            let timestamp_secs = record.timestamp_secs();

            // Convert timestamp to human readable format
            let datetime = chrono::DateTime::from_timestamp(timestamp_secs as i64, 0)
                .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).unwrap());

            println!("{} Record {}: ts={} ({}), price_i64={}, price_f64=${:.9}, size={}, side={}, flags=0x{:02x}, exchange={}, trade_id={}",
                prefix,
                actual_index,
                record.ts_event,
                datetime.format("%Y-%m-%d %H:%M:%S%.9f UTC"),
                record.price,
                price_float,
                record.size,
                record.side_char(),
                record.flags,
                record.exchange,
                record.trade_id
            );

            // Check for anomalies
            if record.price == i64::MAX {
                println!(
                    "    ^^^ EXTREME PRICE: i64::MAX detected ({})",
                    record.price
                );
            }
            if record.price == i64::MIN {
                println!(
                    "    ^^^ EXTREME PRICE: i64::MIN detected ({})",
                    record.price
                );
            }
            if record.size == 0 {
                println!("    ^^^ ZERO SIZE detected");
            }
            if record.size == u32::MAX {
                println!("    ^^^ MAX SIZE detected ({})", record.size);
            }
            if record.ts_event == 0 {
                println!("    ^^^ ZERO TIMESTAMP detected");
            }
            if record.ts_event == u64::MAX {
                println!("    ^^^ MAX TIMESTAMP detected");
            }
        }

        // Focus on the target record
        if let Some(_target_idx) = records.iter().position(|_| {
            start_idx
                + records
                    .iter()
                    .position(|r| std::ptr::eq(r, &records[0]))
                    .unwrap() as u64
                == target_record
        }) {
            // This is a bit convoluted - let me find the target record more directly
            for (i, record) in records.iter().enumerate() {
                let actual_index = start_idx + i as u64;
                if actual_index == target_record {
                    println!("\n=== TARGET RECORD DETAILED ANALYSIS ===");
                    println!("Index: {}", actual_index);
                    println!("Raw bytes analysis:");
                    let bytes = record.to_bytes();
                    println!("  ts_event bytes: {:?}", &bytes[0..8]);
                    println!("  ts_recv bytes:  {:?}", &bytes[8..16]);
                    println!("  price bytes:    {:?}", &bytes[16..24]);
                    println!("  size bytes:     {:?}", &bytes[24..28]);
                    println!(
                        "  side byte:      {:?} ('{}')",
                        bytes[28], bytes[28] as char
                    );
                    println!("  flags byte:     {:?} (0x{:02x})", bytes[29], bytes[29]);
                    println!("  exchange bytes: {:?}", &bytes[30..32]);
                    println!("  trade_id bytes: {:?}", &bytes[32..40]);
                    println!("  reserved bytes: {:?}", &bytes[40..64]);

                    println!("\nParsed values:");
                    println!(
                        "  ts_event: {} ({})",
                        record.ts_event,
                        chrono::DateTime::from_timestamp(record.timestamp_secs() as i64, 0)
                            .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).unwrap())
                            .format("%Y-%m-%d %H:%M:%S%.9f UTC")
                    );
                    println!("  ts_recv: {}", record.ts_recv);
                    println!(
                        "  price: {} nanodollars = ${:.9}",
                        record.price,
                        record.price_as_float()
                    );
                    println!("  size: {} shares", record.size);
                    println!(
                        "  side: {} ('{}') = {}",
                        record.side,
                        record.side_char(),
                        record.side_str()
                    );
                    println!("  flags: 0x{:02x}", record.flags);
                    println!("    - is_odd_lot: {}", record.is_odd_lot());
                    println!("    - is_opening: {}", record.is_opening());
                    println!("    - is_closing: {}", record.is_closing());
                    println!("  exchange: {}", record.exchange);
                    println!("  trade_id: {}", record.trade_id);
                    println!("  calculated_value: ${:.2}", record.value());

                    break;
                }
            }
        }

        println!("\n=== INVESTIGATION COMPLETE ===");
    }
}
