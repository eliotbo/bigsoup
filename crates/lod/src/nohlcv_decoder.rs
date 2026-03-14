//! High-quality decoder for .nohlcv binary files
//!
//! This module provides a robust, efficient decoder for the custom .nohlcv format
//! with improved error handling, performance optimizations, and extensibility.
//!
//! # Features
//! - Zero-copy reading where possible
//! - Robust error handling with detailed error messages
//! - Memory-mapped file support for large datasets (future)
//! - Iterator-based streaming API
//! - Comprehensive validation
//!
//! # Usage Example
//! ```rust,no_run
//! use lod::nohlcv_decoder::{NohlcvReader, OhlcvRecord};
//!
//! fn process_ohlcv_data(path: &str) -> Result<(), Box<dyn std::error::Error>> {
//!     let mut reader = NohlcvReader::open(path)?;
//!
//!     // Stream records efficiently
//!     for record in reader.iter().take(1000) {
//!         let record = record?;
//!         if let Some(close) = record.close() {
//!             println!("Price: {:.2}", close);
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Magic bytes for NOHLCV files
pub const NOHLCV_MAGIC: [u8; 4] = *b"NOHL";

/// Supported versions
pub const SUPPORTED_VERSIONS: [u16; 2] = [1, 2];

/// Size of a single OHLCV record in bytes
const RECORD_SIZE: usize = 64;

/// Custom error type for NOHLCV operations
#[derive(Debug)]
pub enum NohlcvError {
    /// I/O error
    Io(io::Error),
    /// Invalid magic bytes in header
    InvalidMagic { found: [u8; 4], expected: [u8; 4] },
    /// Unsupported version
    UnsupportedVersion { found: u16, supported: Vec<u16> },
    /// UTF-8 decoding error
    InvalidUtf8(std::string::FromUtf8Error),
    /// Data validation error
    InvalidData(String),
    /// Index out of bounds
    IndexOutOfBounds { index: u64, max: u64 },
    /// Corrupted header
    CorruptedHeader(String),
}

impl std::fmt::Display for NohlcvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NohlcvError::Io(e) => write!(f, "I/O error: {}", e),
            NohlcvError::InvalidMagic { found, expected } => {
                write!(
                    f,
                    "Invalid magic bytes: found {:?}, expected {:?}",
                    found, expected
                )
            }
            NohlcvError::UnsupportedVersion { found, supported } => {
                write!(
                    f,
                    "Unsupported version {}, supported: {:?}",
                    found, supported
                )
            }
            NohlcvError::InvalidUtf8(e) => write!(f, "Invalid UTF-8: {}", e),
            NohlcvError::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
            NohlcvError::IndexOutOfBounds { index, max } => {
                write!(f, "Index {} out of bounds (max: {})", index, max)
            }
            NohlcvError::CorruptedHeader(msg) => write!(f, "Corrupted header: {}", msg),
        }
    }
}

impl std::error::Error for NohlcvError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NohlcvError::Io(e) => Some(e),
            NohlcvError::InvalidUtf8(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for NohlcvError {
    fn from(e: io::Error) -> Self {
        NohlcvError::Io(e)
    }
}

impl From<std::string::FromUtf8Error> for NohlcvError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        NohlcvError::InvalidUtf8(e)
    }
}

pub type Result<T> = std::result::Result<T, NohlcvError>;

/// Header structure for .nohlcv files
#[derive(Debug, Clone)]
pub struct NohlcvHeader {
    /// Magic bytes (should be "NOHL")
    pub magic: [u8; 4],
    /// File format version
    pub version: u16,
    /// Total header size in bytes
    pub header_length: u16,
    /// File creation timestamp (nanoseconds since Unix epoch)
    pub created_at_ns: u64,
    /// Total number of records in the file
    pub record_count: u64,
    /// Unique instrument identifier
    pub instrument_id: u32,
    /// Trading symbol
    pub symbol: String,
    /// Bit flags for additional features
    pub footprint_flags: u8,
    /// CRC32 checksum of the header
    pub header_checksum: u32,
    /// Key-value metadata pairs
    pub metadata: BTreeMap<String, String>,
}

impl NohlcvHeader {
    /// Validate header consistency
    pub fn validate(&self) -> Result<()> {
        if self.magic != NOHLCV_MAGIC {
            return Err(NohlcvError::InvalidMagic {
                found: self.magic,
                expected: NOHLCV_MAGIC,
            });
        }

        if !SUPPORTED_VERSIONS.contains(&self.version) {
            return Err(NohlcvError::UnsupportedVersion {
                found: self.version,
                supported: SUPPORTED_VERSIONS.to_vec(),
            });
        }

        if self.symbol.is_empty() {
            return Err(NohlcvError::CorruptedHeader("Empty symbol".to_string()));
        }

        if self.header_length < 32 {
            return Err(NohlcvError::CorruptedHeader(format!(
                "Header too small: {} bytes",
                self.header_length
            )));
        }

        Ok(())
    }

    /// Get the expected file size based on header
    pub fn expected_file_size(&self) -> u64 {
        self.header_length as u64 + (self.record_count * RECORD_SIZE as u64)
    }
}

/// Single OHLCV record (64 bytes)
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct OhlcvRecord {
    /// Event timestamp in nanoseconds since Unix epoch
    pub ts_event: u64,
    /// Opening price (scaled by 1e9)
    pub open_px: i64,
    /// Highest price (scaled by 1e9)
    pub high_px: i64,
    /// Lowest price (scaled by 1e9)
    pub low_px: i64,
    /// Closing price (scaled by 1e9)
    pub close_px: i64,
    /// Trading volume
    pub volume: u64,
    /// Turnover/value traded
    pub turnover: u64,
    /// Number of trades in this bar
    pub trade_count: u32,
    /// Reserved for future use
    #[doc(hidden)]
    pub _reserved: u32,
}

impl OhlcvRecord {
    /// Price scaling factor (1 billion for 9 decimal places)
    const PRICE_SCALE: f64 = 1_000_000_000.0;

    /// Special value indicating null/missing price
    const NULL_PRICE: i64 = i64::MAX;

    /// Create an adjusted copy of this record using a split adjustment factor
    ///
    /// Adjusts OHLC prices by dividing by the factor, preserving NULL values.
    /// Volume and other fields remain unchanged.
    pub fn with_split_adjustment(&self, factor: f64) -> Self {
        use crate::splits::adjust_price_by_factor;

        if factor == 1.0 {
            return *self;
        }

        OhlcvRecord {
            ts_event: self.ts_event,
            open_px: adjust_price_by_factor(self.open_px, factor),
            high_px: adjust_price_by_factor(self.high_px, factor),
            low_px: adjust_price_by_factor(self.low_px, factor),
            close_px: adjust_price_by_factor(self.close_px, factor),
            volume: self.volume,
            turnover: self.turnover,
            trade_count: self.trade_count,
            _reserved: self._reserved,
        }
    }

    /// Decode a record from a 64-byte buffer
    #[inline]
    pub fn from_bytes(buffer: &[u8; 64]) -> Self {
        // Safety: We're reading from a fixed-size array with correct alignment
        OhlcvRecord {
            ts_event: u64::from_le_bytes(buffer[0..8].try_into().unwrap()),
            open_px: i64::from_le_bytes(buffer[8..16].try_into().unwrap()),
            high_px: i64::from_le_bytes(buffer[16..24].try_into().unwrap()),
            low_px: i64::from_le_bytes(buffer[24..32].try_into().unwrap()),
            close_px: i64::from_le_bytes(buffer[32..40].try_into().unwrap()),
            volume: u64::from_le_bytes(buffer[40..48].try_into().unwrap()),
            turnover: u64::from_le_bytes(buffer[48..56].try_into().unwrap()),
            trade_count: u32::from_le_bytes(buffer[56..60].try_into().unwrap()),
            _reserved: u32::from_le_bytes(buffer[60..64].try_into().unwrap()),
        }
    }

    /// Encode the record to a 64-byte buffer
    #[inline]
    pub fn to_bytes(&self) -> [u8; 64] {
        let mut buffer = [0u8; 64];
        buffer[0..8].copy_from_slice(&self.ts_event.to_le_bytes());
        buffer[8..16].copy_from_slice(&self.open_px.to_le_bytes());
        buffer[16..24].copy_from_slice(&self.high_px.to_le_bytes());
        buffer[24..32].copy_from_slice(&self.low_px.to_le_bytes());
        buffer[32..40].copy_from_slice(&self.close_px.to_le_bytes());
        buffer[40..48].copy_from_slice(&self.volume.to_le_bytes());
        buffer[48..56].copy_from_slice(&self.turnover.to_le_bytes());
        buffer[56..60].copy_from_slice(&self.trade_count.to_le_bytes());
        buffer[60..64].copy_from_slice(&self._reserved.to_le_bytes());
        buffer
    }

    /// Convert timestamp to seconds since Unix epoch
    #[inline]
    pub fn timestamp_secs(&self) -> f64 {
        self.ts_event as f64 / Self::PRICE_SCALE
    }

    /// Convert price from scaled integer to float
    #[inline]
    pub fn price_to_f64(price: i64) -> Option<f64> {
        if price == Self::NULL_PRICE || price == -1 {
            None
        } else {
            Some(price as f64 / Self::PRICE_SCALE)
        }
    }

    /// Get open price as float
    #[inline]
    pub fn open(&self) -> Option<f64> {
        Self::price_to_f64(self.open_px)
    }

    /// Get high price as float
    #[inline]
    pub fn high(&self) -> Option<f64> {
        Self::price_to_f64(self.high_px)
    }

    /// Get low price as float
    #[inline]
    pub fn low(&self) -> Option<f64> {
        Self::price_to_f64(self.low_px)
    }

    /// Get close price as float
    #[inline]
    pub fn close(&self) -> Option<f64> {
        Self::price_to_f64(self.close_px)
    }

    /// Check if all price fields are valid
    #[inline]
    pub fn has_valid_prices(&self) -> bool {
        self.open_px != Self::NULL_PRICE
            && self.high_px != Self::NULL_PRICE
            && self.low_px != Self::NULL_PRICE
            && self.close_px != Self::NULL_PRICE
            && self.open_px != -1
            && self.high_px != -1
            && self.low_px != -1
            && self.close_px != -1
    }

    /// Validate OHLC relationships
    #[inline]
    pub fn validate_ohlc(&self) -> bool {
        if !self.has_valid_prices() {
            return false;
        }

        // High should be >= all other prices
        // Low should be <= all other prices
        self.high_px >= self.open_px
            && self.high_px >= self.low_px
            && self.high_px >= self.close_px
            && self.low_px <= self.open_px
            && self.low_px <= self.high_px
            && self.low_px <= self.close_px
    }

    /// Calculate VWAP (Volume Weighted Average Price)
    #[inline]
    pub fn vwap(&self) -> Option<f64> {
        if self.volume > 0 && self.turnover > 0 {
            Some(self.turnover as f64 / self.volume as f64)
        } else {
            None
        }
    }

    /// Calculate typical price (HLC average)
    #[inline]
    pub fn typical_price(&self) -> Option<f64> {
        match (self.high(), self.low(), self.close()) {
            (Some(h), Some(l), Some(c)) => Some((h + l + c) / 3.0),
            _ => None,
        }
    }
}

/// Reader for .nohlcv files with streaming capabilities
pub struct NohlcvReader {
    /// Buffered file reader
    reader: BufReader<File>,
    /// Parsed header
    header: NohlcvHeader,
    /// Byte offset where payload starts
    payload_offset: u64,
    /// File path (for error messages)
    path: PathBuf,
    /// Optional split adjuster for price adjustments
    split_adjuster: Option<crate::splits::SplitAdjuster>,
}

impl NohlcvReader {
    /// Buffer size for efficient reading
    const BUFFER_SIZE: usize = 64 * 1024; // 64KB

    /// Open and parse a .nohlcv file
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_internal(path, false)
    }

    /// Open and parse a .nohlcv file with automatic split discovery
    ///
    /// Automatically looks for `splits.json` in the same directory and loads
    /// splits for the symbol in the file. Warns if splits are not found.
    pub fn open_with_auto_splits(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_internal(path, true)
    }

    /// Internal open implementation
    fn open_internal(path: impl AsRef<Path>, auto_discover_splits: bool) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;

        // Verify file size is reasonable
        let metadata = file.metadata()?;
        if metadata.len() < 32 {
            return Err(NohlcvError::InvalidData(format!(
                "File too small: {} bytes",
                metadata.len()
            )));
        }

        let mut reader = BufReader::with_capacity(Self::BUFFER_SIZE, file);
        let (header, payload_offset) = Self::read_and_validate_header(&mut reader)?;

        // Verify file size matches expectation
        let expected_size = header.expected_file_size();
        if metadata.len() < expected_size {
            return Err(NohlcvError::InvalidData(format!(
                "File truncated: expected {} bytes, found {}",
                expected_size,
                metadata.len()
            )));
        }

        // Auto-discover splits if requested
        let split_adjuster = if auto_discover_splits {
            crate::splits::discover_and_load_splits(&path, &header.symbol, true)
        } else {
            None
        };

        Ok(NohlcvReader {
            reader,
            header,
            payload_offset,
            path,
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
    pub fn header(&self) -> &NohlcvHeader {
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

    /// Get the file path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read a single record by index
    pub fn read_record(&mut self, index: u64) -> Result<OhlcvRecord> {
        if index >= self.header.record_count {
            return Err(NohlcvError::IndexOutOfBounds {
                index,
                max: self.header.record_count - 1,
            });
        }

        let offset = self.payload_offset + (index * RECORD_SIZE as u64);
        self.reader.seek(SeekFrom::Start(offset))?;

        let mut buffer = [0u8; 64];
        self.reader.read_exact(&mut buffer)?;

        let mut record = OhlcvRecord::from_bytes(&buffer);

        // Apply split adjustment if configured
        if let Some(adjuster) = &mut self.split_adjuster {
            let factor = adjuster.factor_for(record.ts_event as i64);
            record = record.with_split_adjustment(factor);
        }

        Ok(record)
    }

    /// Read multiple records starting from an index
    pub fn read_records(&mut self, start: u64, count: usize) -> Result<Vec<OhlcvRecord>> {
        if start >= self.header.record_count {
            return Err(NohlcvError::IndexOutOfBounds {
                index: start,
                max: self.header.record_count - 1,
            });
        }

        let available = (self.header.record_count - start) as usize;
        let to_read = count.min(available);

        // Pre-allocate exact capacity
        let mut records = Vec::with_capacity(to_read);

        let offset = self.payload_offset + (start * RECORD_SIZE as u64);
        self.reader.seek(SeekFrom::Start(offset))?;

        // Read all records in a single batch for efficiency
        let buffer_size = to_read * RECORD_SIZE;
        let mut buffer = vec![0u8; buffer_size];
        self.reader.read_exact(&mut buffer)?;

        // Parse records from buffer
        for chunk in buffer.chunks_exact(RECORD_SIZE) {
            let record_buf: &[u8; 64] = chunk.try_into().unwrap();
            let mut record = OhlcvRecord::from_bytes(record_buf);

            // Apply split adjustment if configured
            if let Some(adjuster) = &mut self.split_adjuster {
                let factor = adjuster.factor_for(record.ts_event as i64);
                record = record.with_split_adjustment(factor);
            }

            records.push(record);
        }

        Ok(records)
    }

    /// Read records with validation
    pub fn read_records_validated(&mut self, start: u64, count: usize) -> Result<Vec<OhlcvRecord>> {
        let records = self.read_records(start, count)?;

        // Filter out invalid records
        Ok(records.into_iter().filter(|r| r.validate_ohlc()).collect())
    }

    /// Read all records (use with caution for large files)
    pub fn read_all(&mut self) -> Result<Vec<OhlcvRecord>> {
        self.read_records(0, self.header.record_count as usize)
    }

    /// Create an iterator over records
    pub fn iter(&mut self) -> RecordIterator<'_> {
        RecordIterator::new(self)
    }

    /// Create an iterator starting from a specific index
    pub fn iter_from(&mut self, start: u64) -> RecordIterator<'_> {
        RecordIterator::from(self, start)
    }

    /// Read and validate header
    fn read_and_validate_header(reader: &mut BufReader<File>) -> Result<(NohlcvHeader, u64)> {
        // Read fixed prefix (8 bytes)
        let mut prefix = [0u8; 8];
        reader.read_exact(&mut prefix)?;

        let magic = [prefix[0], prefix[1], prefix[2], prefix[3]];
        let version = u16::from_le_bytes([prefix[4], prefix[5]]);
        let header_length = u16::from_le_bytes([prefix[6], prefix[7]]) as usize;

        // Validate header size
        if header_length < 32 || header_length > 65535 {
            return Err(NohlcvError::CorruptedHeader(format!(
                "Invalid header length: {}",
                header_length
            )));
        }

        // Read full header
        reader.seek(SeekFrom::Start(0))?;
        let mut header_bytes = vec![0u8; header_length];
        reader.read_exact(&mut header_bytes)?;

        // Parse header fields
        let header = Self::parse_header_bytes(&header_bytes, magic, version, header_length as u16)?;

        // Validate the parsed header
        header.validate()?;

        Ok((header, header_length as u64))
    }

    /// Parse header from bytes
    fn parse_header_bytes(
        header_bytes: &[u8],
        magic: [u8; 4],
        version: u16,
        header_length: u16,
    ) -> Result<NohlcvHeader> {
        let mut offset = 8;

        // Parse fixed fields
        let created_at_ns =
            u64::from_le_bytes(header_bytes[offset..offset + 8].try_into().map_err(|_| {
                NohlcvError::CorruptedHeader("Invalid created_at field".to_string())
            })?);
        offset += 8;

        let record_count =
            u64::from_le_bytes(header_bytes[offset..offset + 8].try_into().map_err(|_| {
                NohlcvError::CorruptedHeader("Invalid record_count field".to_string())
            })?);
        offset += 8;

        let instrument_id =
            u32::from_le_bytes(header_bytes[offset..offset + 4].try_into().map_err(|_| {
                NohlcvError::CorruptedHeader("Invalid instrument_id field".to_string())
            })?);
        offset += 4;

        // Parse symbol with length prefix
        if offset >= header_bytes.len() {
            return Err(NohlcvError::CorruptedHeader(
                "Missing symbol length".to_string(),
            ));
        }
        let symbol_len = header_bytes[offset] as usize;
        offset += 1;

        if offset + symbol_len > header_bytes.len() {
            return Err(NohlcvError::CorruptedHeader(
                "Symbol extends beyond header".to_string(),
            ));
        }
        let symbol = String::from_utf8(header_bytes[offset..offset + symbol_len].to_vec())?;
        offset += symbol_len;

        // Parse remaining fixed fields
        let footprint_flags = if offset < header_bytes.len() {
            header_bytes[offset]
        } else {
            0
        };
        offset += 1;

        let header_checksum = if offset + 4 <= header_bytes.len() {
            u32::from_le_bytes(header_bytes[offset..offset + 4].try_into().unwrap())
        } else {
            0
        };
        offset += 4;

        // Parse metadata if present
        let metadata = if offset < header_bytes.len() {
            Self::decode_metadata(&header_bytes[offset..])?
        } else {
            BTreeMap::new()
        };

        Ok(NohlcvHeader {
            magic,
            version,
            header_length,
            created_at_ns,
            record_count,
            instrument_id,
            symbol,
            footprint_flags,
            header_checksum,
            metadata,
        })
    }

    /// Decode metadata from bytes
    fn decode_metadata(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
        let mut metadata = BTreeMap::new();
        let mut offset = 0;

        while offset + 2 <= bytes.len() {
            // Read key length
            let key_len = bytes[offset] as usize;
            offset += 1;

            if key_len == 0 {
                break; // End of metadata
            }

            if offset + key_len > bytes.len() {
                break; // Incomplete key
            }

            let key = String::from_utf8(bytes[offset..offset + key_len].to_vec())?;
            offset += key_len;

            if offset + 1 >= bytes.len() {
                break; // Missing value
            }

            // Read value length (2 bytes, little-endian)
            let value_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;

            if offset + value_len > bytes.len() {
                break; // Incomplete value
            }

            let value = String::from_utf8(bytes[offset..offset + value_len].to_vec())?;
            offset += value_len;

            metadata.insert(key, value);
        }

        Ok(metadata)
    }
}

/// Iterator for streaming records
pub struct RecordIterator<'a> {
    reader: &'a mut NohlcvReader,
    current_index: u64,
    batch_buffer: Vec<OhlcvRecord>,
    batch_position: usize,
    batch_size: usize,
}

impl<'a> RecordIterator<'a> {
    /// Default batch size for efficient reading
    const DEFAULT_BATCH_SIZE: usize = 1024;

    /// Create a new iterator starting from the beginning
    pub fn new(reader: &'a mut NohlcvReader) -> Self {
        RecordIterator {
            reader,
            current_index: 0,
            batch_buffer: Vec::with_capacity(Self::DEFAULT_BATCH_SIZE),
            batch_position: 0,
            batch_size: Self::DEFAULT_BATCH_SIZE,
        }
    }

    /// Create an iterator starting from a specific index
    pub fn from(reader: &'a mut NohlcvReader, start: u64) -> Self {
        RecordIterator {
            reader,
            current_index: start,
            batch_buffer: Vec::with_capacity(Self::DEFAULT_BATCH_SIZE),
            batch_position: 0,
            batch_size: Self::DEFAULT_BATCH_SIZE,
        }
    }

    /// Set custom batch size for reading
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1);
        self.batch_buffer = Vec::with_capacity(self.batch_size);
        self
    }

    /// Read the next batch of records
    fn read_next_batch(&mut self) -> Result<()> {
        if self.current_index >= self.reader.record_count() {
            return Ok(());
        }

        let remaining = self.reader.record_count() - self.current_index;
        let to_read = self.batch_size.min(remaining as usize);

        self.batch_buffer = self.reader.read_records(self.current_index, to_read)?;
        self.batch_position = 0;
        self.current_index += to_read as u64;

        Ok(())
    }
}

impl<'a> Iterator for RecordIterator<'a> {
    type Item = Result<OhlcvRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        // Check if we need to read the next batch
        if self.batch_position >= self.batch_buffer.len() {
            match self.read_next_batch() {
                Ok(()) => {
                    if self.batch_buffer.is_empty() {
                        return None; // End of file
                    }
                }
                Err(e) => return Some(Err(e)),
            }
        }

        // Return the next record from the batch
        if self.batch_position < self.batch_buffer.len() {
            let record = self.batch_buffer[self.batch_position];
            self.batch_position += 1;
            Some(Ok(record))
        } else {
            None
        }
    }
}

/// Statistics calculator for OHLCV data
#[derive(Debug, Default)]
pub struct OhlcvStats {
    pub record_count: usize,
    pub valid_count: usize,
    pub invalid_count: usize,
    pub min_price: Option<f64>,
    pub max_price: Option<f64>,
    pub avg_close: Option<f64>,
    pub total_volume: u64,
    pub total_turnover: u64,
    pub total_trades: u64,
    pub min_timestamp: Option<u64>,
    pub max_timestamp: Option<u64>,
}

impl OhlcvStats {
    /// Calculate statistics from a slice of records
    pub fn calculate(records: &[OhlcvRecord]) -> Self {
        let mut stats = Self {
            record_count: records.len(),
            ..Default::default()
        };

        if records.is_empty() {
            return stats;
        }

        let mut close_sum = 0.0;
        let mut close_count = 0;

        for record in records {
            // Track timestamp range
            stats.min_timestamp = Some(
                stats
                    .min_timestamp
                    .map_or(record.ts_event, |min| min.min(record.ts_event)),
            );
            stats.max_timestamp = Some(
                stats
                    .max_timestamp
                    .map_or(record.ts_event, |max| max.max(record.ts_event)),
            );

            // Validate and process prices
            if record.validate_ohlc() {
                stats.valid_count += 1;

                // Track price extremes
                if let (Some(low), Some(high)) = (record.low(), record.high()) {
                    stats.min_price = Some(stats.min_price.map_or(low, |min| min.min(low)));
                    stats.max_price = Some(stats.max_price.map_or(high, |max| max.max(high)));
                }

                // Sum for average
                if let Some(close) = record.close() {
                    close_sum += close;
                    close_count += 1;
                }
            } else {
                stats.invalid_count += 1;
            }

            // Aggregate volume metrics
            stats.total_volume += record.volume;
            stats.total_turnover += record.turnover;
            stats.total_trades += record.trade_count as u64;
        }

        // Calculate averages
        if close_count > 0 {
            stats.avg_close = Some(close_sum / close_count as f64);
        }

        stats
    }

    /// Get time range in seconds
    pub fn time_range_secs(&self) -> Option<f64> {
        match (self.min_timestamp, self.max_timestamp) {
            (Some(min), Some(max)) => Some((max - min) as f64 / 1_000_000_000.0),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ohlcv_record_round_trip() {
        let record = OhlcvRecord {
            ts_event: 1234567890000000000,
            open_px: 100_500_000_000,  // $100.50
            high_px: 101_000_000_000,  // $101.00
            low_px: 100_000_000_000,   // $100.00
            close_px: 100_750_000_000, // $100.75
            volume: 1000000,
            turnover: 100500000,
            trade_count: 250,
            _reserved: 0,
        };

        let bytes = record.to_bytes();
        let decoded = OhlcvRecord::from_bytes(&bytes);

        assert_eq!(record, decoded);
    }

    #[test]
    fn test_price_conversion() {
        let price_int = 100_500_000_000i64; // $100.50
        let price_float = OhlcvRecord::price_to_f64(price_int).unwrap();
        assert!((price_float - 100.50).abs() < 0.0001);

        // Test null price
        assert!(OhlcvRecord::price_to_f64(i64::MAX).is_none());
        assert!(OhlcvRecord::price_to_f64(-1).is_none());
    }

    #[test]
    fn test_ohlc_validation() {
        let valid_record = OhlcvRecord {
            ts_event: 1234567890000000000,
            open_px: 100_500_000_000,
            high_px: 101_000_000_000, // High >= all
            low_px: 100_000_000_000,  // Low <= all
            close_px: 100_750_000_000,
            volume: 1000,
            turnover: 100500,
            trade_count: 10,
            _reserved: 0,
        };
        assert!(valid_record.validate_ohlc());

        let invalid_record = OhlcvRecord {
            ts_event: 1234567890000000000,
            open_px: 100_500_000_000,
            high_px: 99_000_000_000, // High < Open (invalid)
            low_px: 100_000_000_000,
            close_px: 100_750_000_000,
            volume: 1000,
            turnover: 100500,
            trade_count: 10,
            _reserved: 0,
        };
        assert!(!invalid_record.validate_ohlc());
    }

    #[test]
    fn test_vwap_calculation() {
        let record = OhlcvRecord {
            ts_event: 0,
            open_px: 100_000_000_000,
            high_px: 100_000_000_000,
            low_px: 100_000_000_000,
            close_px: 100_000_000_000,
            volume: 1000,
            turnover: 100500,
            trade_count: 10,
            _reserved: 0,
        };

        let vwap = record.vwap().unwrap();
        assert!((vwap - 100.5).abs() < 0.01);
    }
}
