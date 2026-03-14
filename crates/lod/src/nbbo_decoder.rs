/// Standalone library for decoding .nbbo binary files
///
/// This module provides a complete decoder for the custom .nbbo format.
/// It can be copied as-is to another crate and used independently.
///
/// # Usage Example
/// ```rust,no_run
/// use lod::nbbo_decoder::{NbboReader, NbboRecord};
///
/// fn read_nbbo_data(path: &str) -> Result<Vec<NbboRecord>, Box<dyn std::error::Error>> {
///     let mut reader = NbboReader::open(path)?;
///     let records = reader.read_records(0, 100)?;
///     Ok(records)
/// }
/// ```
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Magic bytes for NBBO files (NBB0 format)
pub const NBBO_MAGIC: [u8; 4] = *b"NBB0";

/// Expected version
pub const EXPECTED_VERSION: u16 = 1;

/// Custom error type for NBBO operations
#[derive(Debug)]
pub enum NbboError {
    Io(io::Error),
    InvalidMagic([u8; 4]),
    InvalidVersion(u16),
    InvalidUtf8(std::string::FromUtf8Error),
    InvalidData(String),
}

impl std::fmt::Display for NbboError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NbboError::Io(e) => write!(f, "IO error: {}", e),
            NbboError::InvalidMagic(magic) => write!(f, "Invalid magic bytes: {:?}", magic),
            NbboError::InvalidVersion(v) => write!(f, "Invalid version: {}", v),
            NbboError::InvalidUtf8(e) => write!(f, "Invalid UTF-8: {}", e),
            NbboError::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
        }
    }
}

impl std::error::Error for NbboError {}

impl From<io::Error> for NbboError {
    fn from(e: io::Error) -> Self {
        NbboError::Io(e)
    }
}

impl From<std::string::FromUtf8Error> for NbboError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        NbboError::InvalidUtf8(e)
    }
}

pub type Result<T> = std::result::Result<T, NbboError>;

/// Header structure for .nbbo files
#[derive(Debug, Clone)]
pub struct NbboHeader {
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

/// Single NBBO record (48 bytes)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NbboRecord {
    /// Timestamp in nanoseconds since Unix epoch
    pub ts_event: u64,
    /// Receive timestamp in nanoseconds
    pub ts_recv: u64,
    /// Best bid price (scaled integer, typically 9 decimal places), None if no bid
    pub bid_px: Option<i64>,
    /// Bid size/quantity
    pub bid_sz: u32,
    /// Bid count
    pub bid_ct: u32,
    /// Best ask price (scaled integer), None if no ask
    pub ask_px: Option<i64>,
    /// Ask size/quantity
    pub ask_sz: u32,
    /// Ask count
    pub ask_ct: u32,
}

impl NbboRecord {
    /// Create an adjusted copy of this record using a split adjustment factor
    ///
    /// Adjusts bid and ask prices by dividing by the factor, preserving None values.
    /// Sizes and other fields remain unchanged.
    pub fn with_split_adjustment(&self, factor: f64) -> Self {
        use crate::splits::adjust_price_by_factor;

        if factor == 1.0 {
            return *self;
        }

        NbboRecord {
            ts_event: self.ts_event,
            ts_recv: self.ts_recv,
            bid_px: self.bid_px.map(|px| adjust_price_by_factor(px, factor)),
            bid_sz: self.bid_sz,
            bid_ct: self.bid_ct,
            ask_px: self.ask_px.map(|px| adjust_price_by_factor(px, factor)),
            ask_sz: self.ask_sz,
            ask_ct: self.ask_ct,
        }
    }

    /// Decode a record from a 48-byte buffer
    pub fn from_bytes(buffer: &[u8; 48]) -> Self {
        let ts_event = u64::from_le_bytes(buffer[0..8].try_into().unwrap());
        let ts_recv = u64::from_le_bytes(buffer[8..16].try_into().unwrap());

        let bid_px_raw = i64::from_le_bytes(buffer[16..24].try_into().unwrap());
        let bid_px = if bid_px_raw == -1 {
            None
        } else {
            Some(bid_px_raw)
        };

        let bid_sz = u32::from_le_bytes(buffer[24..28].try_into().unwrap());
        let bid_ct = u32::from_le_bytes(buffer[28..32].try_into().unwrap());

        let ask_px_raw = i64::from_le_bytes(buffer[32..40].try_into().unwrap());
        let ask_px = if ask_px_raw == -1 {
            None
        } else {
            Some(ask_px_raw)
        };

        let ask_sz = u32::from_le_bytes(buffer[40..44].try_into().unwrap());
        let ask_ct = u32::from_le_bytes(buffer[44..48].try_into().unwrap());

        NbboRecord {
            ts_event,
            ts_recv,
            bid_px,
            bid_sz,
            bid_ct,
            ask_px,
            ask_sz,
            ask_ct,
        }
    }

    /// Encode the record to a 48-byte buffer
    pub fn to_bytes(&self) -> [u8; 48] {
        let mut buffer = [0u8; 48];

        buffer[0..8].copy_from_slice(&self.ts_event.to_le_bytes());
        buffer[8..16].copy_from_slice(&self.ts_recv.to_le_bytes());

        let bid_px_raw = self.bid_px.unwrap_or(-1);
        buffer[16..24].copy_from_slice(&bid_px_raw.to_le_bytes());
        buffer[24..28].copy_from_slice(&self.bid_sz.to_le_bytes());
        buffer[28..32].copy_from_slice(&self.bid_ct.to_le_bytes());

        let ask_px_raw = self.ask_px.unwrap_or(-1);
        buffer[32..40].copy_from_slice(&ask_px_raw.to_le_bytes());
        buffer[40..44].copy_from_slice(&self.ask_sz.to_le_bytes());
        buffer[44..48].copy_from_slice(&self.ask_ct.to_le_bytes());

        buffer
    }

    /// Convert timestamp to seconds since Unix epoch
    pub fn timestamp_secs(&self) -> f64 {
        self.ts_event as f64 / 1_000_000_000.0
    }

    /// Convert price from scaled integer to float
    /// Assumes prices are stored with 9 decimal places of precision
    pub fn price_to_f64(price: i64) -> f64 {
        price as f64 / 1_000_000_000.0
    }

    /// Get bid price as float
    pub fn bid(&self) -> Option<f64> {
        self.bid_px.map(Self::price_to_f64)
    }

    /// Get ask price as float
    pub fn ask(&self) -> Option<f64> {
        self.ask_px.map(Self::price_to_f64)
    }

    /// Calculate bid-ask spread
    pub fn spread(&self) -> Option<f64> {
        match (self.bid_px, self.ask_px) {
            (Some(bid), Some(ask)) => Some(Self::price_to_f64(ask - bid)),
            _ => None,
        }
    }

    /// Calculate mid price (average of bid and ask)
    pub fn mid_price(&self) -> Option<f64> {
        match (self.bid_px, self.ask_px) {
            (Some(bid), Some(ask)) => Some(Self::price_to_f64((bid + ask) / 2)),
            _ => None,
        }
    }

    /// Check if both bid and ask are present (two-sided market)
    pub fn is_two_sided(&self) -> bool {
        self.bid_px.is_some() && self.ask_px.is_some()
    }

    /// Check if only bid is present
    pub fn is_bid_only(&self) -> bool {
        self.bid_px.is_some() && self.ask_px.is_none()
    }

    /// Check if only ask is present
    pub fn is_ask_only(&self) -> bool {
        self.bid_px.is_none() && self.ask_px.is_some()
    }

    /// Check if neither bid nor ask is present
    pub fn is_empty(&self) -> bool {
        self.bid_px.is_none() && self.ask_px.is_none()
    }
}

/// Reader for .nbbo files
pub struct NbboReader {
    file: File,
    header: NbboHeader,
    payload_offset: u64,
    /// Optional split adjuster for price adjustments
    split_adjuster: Option<crate::splits::SplitAdjuster>,
}

impl NbboReader {
    /// Open and parse a .nbbo file
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_internal(path, false)
    }

    /// Open and parse a .nbbo file with automatic split discovery
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

        Ok(NbboReader {
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
    pub fn header(&self) -> &NbboHeader {
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
    pub fn read_record(&mut self, index: u64) -> Result<NbboRecord> {
        if index >= self.header.record_count {
            return Err(NbboError::InvalidData(format!(
                "Index {} out of bounds (max: {})",
                index,
                self.header.record_count - 1
            )));
        }

        let offset = self.payload_offset + (index * 48);
        self.file.seek(SeekFrom::Start(offset))?;

        let mut buffer = [0u8; 48];
        self.file.read_exact(&mut buffer)?;

        let mut record = NbboRecord::from_bytes(&buffer);

        // Apply split adjustment if configured
        if let Some(adjuster) = &mut self.split_adjuster {
            let factor = adjuster.factor_for(record.ts_event as i64);
            record = record.with_split_adjustment(factor);
        }

        Ok(record)
    }

    /// Read multiple records starting from an index
    pub fn read_records(&mut self, start: u64, count: usize) -> Result<Vec<NbboRecord>> {
        if start >= self.header.record_count {
            return Err(NbboError::InvalidData(format!(
                "Start index {} out of bounds",
                start
            )));
        }

        let available = (self.header.record_count - start) as usize;
        let to_read = count.min(available);

        let mut records = Vec::with_capacity(to_read);
        let offset = self.payload_offset + (start * 48);
        self.file.seek(SeekFrom::Start(offset))?;

        for _ in 0..to_read {
            let mut buffer = [0u8; 48];
            self.file.read_exact(&mut buffer)?;
            let mut record = NbboRecord::from_bytes(&buffer);

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
    pub fn read_all(&mut self) -> Result<Vec<NbboRecord>> {
        self.read_records(0, self.header.record_count as usize)
    }

    /// Filter records by criteria
    pub fn read_filtered<F>(
        &mut self,
        start: u64,
        count: usize,
        predicate: F,
    ) -> Result<Vec<NbboRecord>>
    where
        F: Fn(&NbboRecord) -> bool,
    {
        let records = self.read_records(start, count)?;
        Ok(records.into_iter().filter(predicate).collect())
    }

    /// Read only two-sided quotes
    pub fn read_two_sided(&mut self, start: u64, count: usize) -> Result<Vec<NbboRecord>> {
        self.read_filtered(start, count, |r| r.is_two_sided())
    }

    /// Read header from a file
    fn read_header(file: &mut File) -> Result<(NbboHeader, u64)> {
        // Read fixed prefix (8 bytes)
        let mut prefix = [0u8; 8];
        file.read_exact(&mut prefix)?;

        let magic = [prefix[0], prefix[1], prefix[2], prefix[3]];
        if magic != NBBO_MAGIC {
            return Err(NbboError::InvalidMagic(magic));
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

        let header = NbboHeader {
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

        while offset + 2 <= bytes.len() {
            let key_len = bytes[offset] as usize;
            offset += 1;

            if key_len == 0 || offset + key_len > bytes.len() {
                break;
            }

            let key = String::from_utf8(bytes[offset..offset + key_len].to_vec())?;
            offset += key_len;

            if offset >= bytes.len() {
                break;
            }

            let value_len = bytes[offset] as usize;
            offset += 1;

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
    reader: &'a mut NbboReader,
    current: u64,
}

impl<'a> RecordIterator<'a> {
    pub fn new(reader: &'a mut NbboReader) -> Self {
        RecordIterator { reader, current: 0 }
    }
}

impl<'a> Iterator for RecordIterator<'a> {
    type Item = Result<NbboRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.reader.record_count() {
            return None;
        }

        let result = self.reader.read_record(self.current);
        self.current += 1;
        Some(result)
    }
}

/// Statistics calculator for NBBO data
#[derive(Debug, Default)]
pub struct NbboStats {
    pub record_count: usize,
    pub two_sided_count: usize,
    pub bid_only_count: usize,
    pub ask_only_count: usize,
    pub empty_count: usize,
    pub min_bid: Option<f64>,
    pub max_bid: Option<f64>,
    pub min_ask: Option<f64>,
    pub max_ask: Option<f64>,
    pub min_spread: Option<f64>,
    pub max_spread: Option<f64>,
    pub avg_spread: Option<f64>,
    pub total_bid_size: u64,
    pub total_ask_size: u64,
}

impl NbboStats {
    pub fn calculate(records: &[NbboRecord]) -> Self {
        let mut stats = Self {
            record_count: records.len(),
            ..Default::default()
        };

        if records.is_empty() {
            return stats;
        }

        let mut spread_sum = 0.0;
        let mut spread_count = 0;

        for record in records {
            // Categorize records
            if record.is_two_sided() {
                stats.two_sided_count += 1;
            } else if record.is_bid_only() {
                stats.bid_only_count += 1;
            } else if record.is_ask_only() {
                stats.ask_only_count += 1;
            } else {
                stats.empty_count += 1;
            }

            // Bid statistics
            if let Some(bid) = record.bid() {
                stats.min_bid = Some(stats.min_bid.map_or(bid, |min| min.min(bid)));
                stats.max_bid = Some(stats.max_bid.map_or(bid, |max| max.max(bid)));
                stats.total_bid_size += record.bid_sz as u64;
            }

            // Ask statistics
            if let Some(ask) = record.ask() {
                stats.min_ask = Some(stats.min_ask.map_or(ask, |min| min.min(ask)));
                stats.max_ask = Some(stats.max_ask.map_or(ask, |max| max.max(ask)));
                stats.total_ask_size += record.ask_sz as u64;
            }

            // Spread statistics
            if let Some(spread) = record.spread() {
                stats.min_spread = Some(stats.min_spread.map_or(spread, |min| min.min(spread)));
                stats.max_spread = Some(stats.max_spread.map_or(spread, |max| max.max(spread)));
                spread_sum += spread;
                spread_count += 1;
            }
        }

        if spread_count > 0 {
            stats.avg_spread = Some(spread_sum / spread_count as f64);
        }

        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nbbo_record_round_trip() {
        let record = NbboRecord {
            ts_event: 1234567890000000000,
            ts_recv: 1234567890000000001,
            bid_px: Some(25500000000), // $25.50
            bid_sz: 100,
            bid_ct: 1,
            ask_px: Some(25520000000), // $25.52
            ask_sz: 200,
            ask_ct: 2,
        };

        let bytes = record.to_bytes();
        let decoded = NbboRecord::from_bytes(&bytes);

        assert_eq!(record, decoded);
    }

    #[test]
    fn test_price_conversion() {
        let price_int = 25500000000i64; // $25.50
        let price_float = NbboRecord::price_to_f64(price_int);
        assert!((price_float - 25.50).abs() < 0.0001);
    }

    #[test]
    fn test_spread_calculation() {
        let record = NbboRecord {
            ts_event: 0,
            ts_recv: 0,
            bid_px: Some(25500000000), // $25.50
            bid_sz: 100,
            bid_ct: 1,
            ask_px: Some(25520000000), // $25.52
            ask_sz: 200,
            ask_ct: 2,
        };

        let spread = record.spread().unwrap();
        assert!((spread - 0.02).abs() < 0.0001); // $0.02 spread
    }

    #[test]
    fn test_mid_price() {
        let record = NbboRecord {
            ts_event: 0,
            ts_recv: 0,
            bid_px: Some(25500000000), // $25.50
            bid_sz: 100,
            bid_ct: 1,
            ask_px: Some(25520000000), // $25.52
            ask_sz: 200,
            ask_ct: 2,
        };

        let mid = record.mid_price().unwrap();
        assert!((mid - 25.51).abs() < 0.0001); // $25.51 mid
    }
}
