/// Memory-mapped NTRD file reader for zero-copy access
///
/// This module extends the NTRD decoder with memory-mapped file support,
/// enabling efficient zero-copy access to large trade datasets.
use crate::ntrd_decoder::{
    NtrdError, NtrdHeader, Result, TradeRecord, EXPECTED_VERSION, NTRD_MAGIC,
};
use std::path::Path;

#[cfg(feature = "zerocopy")]
use zerocopy::{AsBytes, FromBytes, FromZeroes};

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
use crate::mmap::MmappedFile;

/// Zero-copy compatible trade record
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "zerocopy", derive(FromZeroes, FromBytes, AsBytes))]
pub struct TradeRecordZC {
    pub ts_event: u64,
    pub ts_recv: u64,
    pub price: i64,
    pub size: u32,
    pub side: u8,
    pub flags: u8,
    pub exchange: u16,
    pub trade_id: u64,
    _reserved: [u8; 24], // Padding to reach 64 bytes
}

impl TradeRecordZC {
    /// Convert to regular TradeRecord
    pub fn to_record(&self) -> TradeRecord {
        TradeRecord {
            ts_event: self.ts_event,
            ts_recv: self.ts_recv,
            price: self.price,
            size: self.size,
            side: self.side,
            flags: self.flags,
            exchange: self.exchange,
            trade_id: self.trade_id,
        }
    }

    /// Create from regular TradeRecord
    pub fn from_record(record: &TradeRecord) -> Self {
        TradeRecordZC {
            ts_event: record.ts_event,
            ts_recv: record.ts_recv,
            price: record.price,
            size: record.size,
            side: record.side,
            flags: record.flags,
            exchange: record.exchange,
            trade_id: record.trade_id,
            _reserved: [0u8; 24],
        }
    }
}

/// Memory-mapped NTRD file reader
#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub struct NtrdMmapReader {
    mmap: MmappedFile,
    header: NtrdHeader,
    data_offset: usize,
    record_count: u64,
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl NtrdMmapReader {
    /// Open an NTRD file with memory mapping
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mmap = MmappedFile::open(path)
            .map_err(|e| NtrdError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Read and validate header
        if mmap.len() < 8 {
            return Err(NtrdError::InvalidData(
                "File too small for header".to_string(),
            ));
        }

        let magic = &mmap.as_bytes()[0..4];
        if magic != NTRD_MAGIC {
            let mut magic_array = [0u8; 4];
            magic_array.copy_from_slice(magic);
            return Err(NtrdError::InvalidMagic(magic_array));
        }

        let version = u16::from_le_bytes([mmap.as_bytes()[4], mmap.as_bytes()[5]]);
        if version != EXPECTED_VERSION {
            eprintln!(
                "Warning: Version {} (expected {})",
                version, EXPECTED_VERSION
            );
        }

        let header_length = u16::from_le_bytes([mmap.as_bytes()[6], mmap.as_bytes()[7]]) as usize;

        // Parse full header
        let created_at_ns = u64::from_le_bytes(mmap.as_bytes()[8..16].try_into().unwrap());
        let record_count = u64::from_le_bytes(mmap.as_bytes()[16..24].try_into().unwrap());
        let instrument_id = u32::from_le_bytes(mmap.as_bytes()[24..28].try_into().unwrap());

        // Extract symbol
        let symbol_len = mmap.as_bytes()[28] as usize;
        let symbol = String::from_utf8(mmap.as_bytes()[29..29 + symbol_len].to_vec())?;

        let footprint_flags = mmap.as_bytes()[29 + symbol_len];
        let header_checksum = u32::from_le_bytes(
            mmap.as_bytes()[30 + symbol_len..34 + symbol_len]
                .try_into()
                .unwrap(),
        );

        let header = NtrdHeader {
            magic: NTRD_MAGIC,
            version,
            header_length: header_length as u16,
            created_at_ns,
            record_count,
            instrument_id,
            symbol,
            footprint_flags,
            header_checksum,
            metadata: Default::default(),
        };

        Ok(NtrdMmapReader {
            mmap,
            header,
            data_offset: header_length,
            record_count,
        })
    }

    /// Get the header
    pub fn header(&self) -> &NtrdHeader {
        &self.header
    }

    /// Get a single record (zero-copy, converted to an owned value)
    pub fn get_record(&self, index: u64) -> Option<TradeRecord> {
        if index >= self.record_count {
            return None;
        }

        let offset = self.data_offset + (index as usize * std::mem::size_of::<TradeRecordZC>());
        self.mmap
            .cast::<TradeRecordZC>(offset)
            .map(|r| r.to_record())
    }

    /// Get a single zero-copy record reference
    pub fn get_record_zc(&self, index: u64) -> Option<&TradeRecordZC> {
        if index >= self.record_count {
            return None;
        }

        let offset = self.data_offset + (index as usize * std::mem::size_of::<TradeRecordZC>());
        self.mmap.cast::<TradeRecordZC>(offset)
    }

    /// Get a slice of records (zero-copy)
    pub fn get_records(&self, start: u64, count: u64) -> Option<Vec<TradeRecord>> {
        if start + count > self.record_count {
            return None;
        }

        let offset = self.data_offset + (start as usize * std::mem::size_of::<TradeRecordZC>());
        self.mmap
            .cast_slice::<TradeRecordZC>(offset, count as usize)
            .map(|slice| slice.iter().map(|r| r.to_record()).collect())
    }

    /// Get a zero-copy slice of records
    pub fn get_records_zc(&self, start: u64, count: u64) -> Option<&[TradeRecordZC]> {
        if start + count > self.record_count {
            return None;
        }

        let offset = self.data_offset + (start as usize * std::mem::size_of::<TradeRecordZC>());
        self.mmap
            .cast_slice::<TradeRecordZC>(offset, count as usize)
    }

    /// Binary search for timestamp
    pub fn find_timestamp(&self, target_ts: u64) -> std::result::Result<u64, u64> {
        let mut left = 0;
        let mut right = self.record_count;

        while left < right {
            let mid = left + (right - left) / 2;

            if let Some(record) = self.get_record(mid) {
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
    pub fn iter_range(&self, start: u64, end: u64) -> NtrdMmapIter<'_> {
        NtrdMmapIter {
            reader: self,
            current: start,
            end: end.min(self.record_count),
        }
    }

    /// Create an iterator over all records
    pub fn iter(&self) -> NtrdMmapIter<'_> {
        self.iter_range(0, self.record_count)
    }

    /// Get all records as a zero-copy slice (if possible)
    pub fn as_slice(&self) -> Option<&[TradeRecordZC]> {
        self.get_records_zc(0, self.record_count)
    }
}

/// Iterator for memory-mapped NTRD records
#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub struct NtrdMmapIter<'a> {
    reader: &'a NtrdMmapReader,
    current: u64,
    end: u64,
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl<'a> Iterator for NtrdMmapIter<'a> {
    type Item = TradeRecord;

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
impl<'a> ExactSizeIterator for NtrdMmapIter<'a> {
    fn len(&self) -> usize {
        (self.end - self.current) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use zerocopy::AsBytes;

    #[test]
    fn test_trade_record_conversion() {
        let original = TradeRecord {
            ts_event: 1000,
            ts_recv: 1001,
            price: 100_000_000_000, // $100.00
            size: 100,
            side: b'B',
            flags: 0x00,
            exchange: 1,
            trade_id: 12345,
        };

        let zc = TradeRecordZC::from_record(&original);
        let converted = zc.to_record();

        assert_eq!(original, converted);
    }

    #[test]
    fn test_ntrd_zero_copy_slice_access() {
        let mut file = NamedTempFile::new().expect("create temp file");

        let mut header = Vec::new();
        header.extend_from_slice(&NTRD_MAGIC);
        header.extend_from_slice(&EXPECTED_VERSION.to_le_bytes());

        let header_length: u16 = 64;
        header.extend_from_slice(&header_length.to_le_bytes());
        header.extend_from_slice(&1_u64.to_le_bytes()); // created_at_ns
        header.extend_from_slice(&2_u64.to_le_bytes()); // record_count
        header.extend_from_slice(&123_u32.to_le_bytes()); // instrument_id
        header.push(4); // symbol length
        header.extend_from_slice(b"TEST");
        header.push(0); // footprint_flags
        header.extend_from_slice(&0_u32.to_le_bytes()); // header_checksum

        while header.len() < header_length as usize {
            header.push(0);
        }

        file.write_all(&header).expect("write header");

        let records = [
            TradeRecord {
                ts_event: 10,
                ts_recv: 11,
                price: 100_000_000_000,
                size: 100,
                side: b'B',
                flags: 0x00,
                exchange: 1,
                trade_id: 1,
            },
            TradeRecord {
                ts_event: 20,
                ts_recv: 21,
                price: 101_000_000_000,
                size: 150,
                side: b'S',
                flags: 0x00,
                exchange: 1,
                trade_id: 2,
            },
        ];

        for record in &records {
            let zc = TradeRecordZC::from_record(record);
            file.write_all(zc.as_bytes()).expect("write record");
        }

        file.flush().expect("flush temp file");

        let reader = NtrdMmapReader::open(file.path()).expect("open mmap reader");

        let slice = reader.get_records_zc(0, 2).expect("zero-copy slice");
        assert_eq!(slice.len(), 2);
        assert_eq!(slice[0].ts_event, 10);
        assert_eq!(slice[1].ts_event, 20);

        let single = reader.get_record_zc(1).expect("record ref");
        assert!(std::ptr::eq(single, &slice[1]));
    }
}
