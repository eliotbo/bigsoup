/// Memory-mapped NOHLCV file reader for zero-copy access
///
/// This module extends the NOHLCV decoder with memory-mapped file support,
/// enabling efficient zero-copy access to large OHLCV datasets.
use crate::nohlcv_decoder::{
    NohlcvError, NohlcvHeader, OhlcvRecord, Result, NOHLCV_MAGIC, SUPPORTED_VERSIONS,
};
use std::path::Path;

#[cfg(feature = "zerocopy")]
use zerocopy::{FromBytes, FromZeroes};

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
use crate::mmap::MmappedFile;

/// Zero-copy compatible OHLCV record (matches binary layout)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "zerocopy", derive(FromZeroes, FromBytes))]
pub struct OhlcvRecordZC {
    pub ts_event: u64,
    pub open_px: i64,
    pub high_px: i64,
    pub low_px: i64,
    pub close_px: i64,
    pub volume: u64,
    pub trade_count: u32,
    pub turnover: u64,
    pub _padding: u32, // Padding to make it 64 bytes
}

impl OhlcvRecordZC {
    /// Convert to regular OhlcvRecord
    pub fn to_record(&self) -> OhlcvRecord {
        OhlcvRecord {
            ts_event: self.ts_event,
            open_px: self.open_px,
            high_px: self.high_px,
            low_px: self.low_px,
            close_px: self.close_px,
            volume: self.volume,
            turnover: self.turnover,
            trade_count: self.trade_count,
            _reserved: self._padding,
        }
    }

    /// Create from regular OhlcvRecord
    pub fn from_record(record: &OhlcvRecord) -> Self {
        OhlcvRecordZC {
            ts_event: record.ts_event,
            open_px: record.open_px,
            high_px: record.high_px,
            low_px: record.low_px,
            close_px: record.close_px,
            volume: record.volume,
            trade_count: record.trade_count,
            turnover: record.turnover,
            _padding: record._reserved,
        }
    }

    /// Get timestamp in seconds
    pub fn timestamp_secs(&self) -> f64 {
        self.ts_event as f64 / 1_000_000_000.0
    }

    /// Check if this is a valid record
    pub fn is_valid(&self) -> bool {
        self.ts_event > 0
            && self.high_px >= self.low_px
            && self.high_px >= self.open_px
            && self.high_px >= self.close_px
            && self.low_px <= self.open_px
            && self.low_px <= self.close_px
    }
}

/// Memory-mapped NOHLCV file reader
#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub struct NohlcvMmapReader {
    mmap: MmappedFile,
    header: NohlcvHeader,
    data_offset: usize,
    record_count: u64,
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl NohlcvMmapReader {
    /// Open a NOHLCV file with memory mapping
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mmap = MmappedFile::open(path)
            .map_err(|e| NohlcvError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Read and validate header
        if mmap.len() < 8 {
            return Err(NohlcvError::InvalidData(
                "File too small for header".to_string(),
            ));
        }

        let magic = &mmap.as_bytes()[0..4];
        let mut magic_array = [0u8; 4];
        magic_array.copy_from_slice(magic);

        if magic_array != NOHLCV_MAGIC {
            return Err(NohlcvError::InvalidMagic {
                found: magic_array,
                expected: NOHLCV_MAGIC,
            });
        }

        let version = u16::from_le_bytes([mmap.as_bytes()[4], mmap.as_bytes()[5]]);
        if !SUPPORTED_VERSIONS.contains(&version) {
            return Err(NohlcvError::UnsupportedVersion {
                found: version,
                supported: SUPPORTED_VERSIONS.to_vec(),
            });
        }

        let header_length = u16::from_le_bytes([mmap.as_bytes()[6], mmap.as_bytes()[7]]) as usize;

        // Parse header fields
        let created_at_ns = u64::from_le_bytes(mmap.as_bytes()[8..16].try_into().unwrap());
        let record_count = u64::from_le_bytes(mmap.as_bytes()[16..24].try_into().unwrap());
        let instrument_id = u32::from_le_bytes(mmap.as_bytes()[24..28].try_into().unwrap());

        // Extract symbol
        let symbol_len = mmap.as_bytes()[28] as usize;
        let symbol = String::from_utf8(mmap.as_bytes()[29..29 + symbol_len].to_vec())
            .map_err(NohlcvError::InvalidUtf8)?;

        let header = NohlcvHeader {
            magic: NOHLCV_MAGIC,
            version,
            header_length: header_length as u16,
            created_at_ns,
            record_count,
            instrument_id,
            symbol,
            footprint_flags: 0,
            header_checksum: 0,
            metadata: Default::default(),
        };

        Ok(NohlcvMmapReader {
            mmap,
            header,
            data_offset: header_length,
            record_count,
        })
    }

    /// Get the header
    pub fn header(&self) -> &NohlcvHeader {
        &self.header
    }

    /// Get total number of records
    pub fn len(&self) -> u64 {
        self.record_count
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.record_count == 0
    }

    /// Get a single record (zero-copy)
    pub fn get_record(&self, index: u64) -> Option<OhlcvRecord> {
        if index >= self.record_count {
            return None;
        }

        let offset = self.data_offset + (index as usize * std::mem::size_of::<OhlcvRecordZC>());
        self.mmap
            .cast::<OhlcvRecordZC>(offset)
            .map(|r| r.to_record())
    }

    /// Get a raw zero-copy record
    pub fn get_record_zc(&self, index: u64) -> Option<&OhlcvRecordZC> {
        if index >= self.record_count {
            return None;
        }

        let offset = self.data_offset + (index as usize * std::mem::size_of::<OhlcvRecordZC>());
        self.mmap.cast::<OhlcvRecordZC>(offset)
    }

    /// Get a slice of records (zero-copy)
    pub fn get_records(&self, start: u64, count: u64) -> Option<Vec<OhlcvRecord>> {
        if start + count > self.record_count {
            return None;
        }

        let offset = self.data_offset + (start as usize * std::mem::size_of::<OhlcvRecordZC>());
        self.mmap
            .cast_slice::<OhlcvRecordZC>(offset, count as usize)
            .map(|slice| slice.iter().map(|r| r.to_record()).collect())
    }

    /// Get a zero-copy slice of records
    pub fn get_records_zc(&self, start: u64, count: u64) -> Option<&[OhlcvRecordZC]> {
        if start + count > self.record_count {
            return None;
        }

        let offset = self.data_offset + (start as usize * std::mem::size_of::<OhlcvRecordZC>());
        self.mmap
            .cast_slice::<OhlcvRecordZC>(offset, count as usize)
    }

    /// Binary search for timestamp
    pub fn find_timestamp(&self, target_ts: u64) -> std::result::Result<u64, u64> {
        let mut left = 0;
        let mut right = self.record_count;

        while left < right {
            let mid = left + (right - left) / 2;

            if let Some(record) = self.get_record_zc(mid) {
                match record.ts_event.cmp(&target_ts) {
                    std::cmp::Ordering::Less => left = mid + 1,
                    std::cmp::Ordering::Greater => right = mid,
                    std::cmp::Ordering::Equal => return Ok(mid),
                }
            } else {
                break;
            }
        }

        Err(left)
    }

    /// Find range of records within time bounds
    pub fn find_time_range(&self, start_ts: u64, end_ts: u64) -> Option<(u64, u64)> {
        let start_idx = self.find_timestamp(start_ts).unwrap_or_else(|idx| idx);

        let end_idx = self
            .find_timestamp(end_ts)
            .map(|idx| idx + 1)
            .unwrap_or_else(|idx| idx);

        if start_idx < end_idx && end_idx <= self.record_count {
            Some((start_idx, end_idx))
        } else {
            None
        }
    }

    /// Create an iterator over a range of records
    pub fn iter_range(&self, start: u64, end: u64) -> NohlcvMmapIter<'_> {
        NohlcvMmapIter {
            reader: self,
            current: start,
            end: end.min(self.record_count),
        }
    }

    /// Create an iterator over all records
    pub fn iter(&self) -> NohlcvMmapIter<'_> {
        self.iter_range(0, self.record_count)
    }

    /// Get all records as a zero-copy slice (if possible)
    pub fn as_slice(&self) -> Option<&[OhlcvRecordZC]> {
        self.get_records_zc(0, self.record_count)
    }
}

/// Iterator for memory-mapped NOHLCV records
#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub struct NohlcvMmapIter<'a> {
    reader: &'a NohlcvMmapReader,
    current: u64,
    end: u64,
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl<'a> Iterator for NohlcvMmapIter<'a> {
    type Item = OhlcvRecord;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.end {
            return None;
        }

        let record = self.reader.get_record(self.current);
        self.current += 1;
        record
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.end - self.current) as usize;
        (remaining, Some(remaining))
    }
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl<'a> ExactSizeIterator for NohlcvMmapIter<'a> {
    fn len(&self) -> usize {
        (self.end - self.current) as usize
    }
}

/// Zero-copy iterator that returns references
#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub struct NohlcvMmapIterZC<'a> {
    slice: &'a [OhlcvRecordZC],
    current: usize,
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl<'a> NohlcvMmapIterZC<'a> {
    pub fn new(reader: &'a NohlcvMmapReader) -> Option<Self> {
        reader
            .as_slice()
            .map(|slice| NohlcvMmapIterZC { slice, current: 0 })
    }
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl<'a> Iterator for NohlcvMmapIterZC<'a> {
    type Item = &'a OhlcvRecordZC;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.slice.len() {
            return None;
        }

        let item = &self.slice[self.current];
        self.current += 1;
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.slice.len() - self.current;
        (remaining, Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ohlcv_record_conversion() {
        let original = OhlcvRecord {
            ts_event: 1_000_000_000,
            open_px: 100_000_000_000,
            high_px: 101_000_000_000,
            low_px: 99_000_000_000,
            close_px: 100_500_000_000,
            volume: 1000,
            turnover: 100000,
            trade_count: 10,
            _reserved: 0,
        };

        let zc = OhlcvRecordZC::from_record(&original);
        let converted = zc.to_record();

        assert_eq!(original, converted);
        assert!(zc.is_valid());
    }

    #[test]
    fn test_invalid_ohlcv_record() {
        let invalid = OhlcvRecordZC {
            ts_event: 1_000_000_000,
            open_px: 100_000_000_000,
            high_px: 99_000_000_000, // High < Low (invalid)
            low_px: 101_000_000_000,
            close_px: 100_500_000_000,
            volume: 1000,
            trade_count: 10,
            turnover: 0,
            _padding: 0,
        };

        assert!(!invalid.is_valid());
    }
}
