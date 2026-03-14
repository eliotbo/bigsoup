/// Memory-mapped NBBO file reader for zero-copy access
///
/// This module extends the NBBO decoder with memory-mapped file support,
/// enabling efficient zero-copy access to large NBBO datasets.
use crate::nbbo_decoder::{
    NbboError, NbboHeader, NbboRecord, Result, EXPECTED_VERSION, NBBO_MAGIC,
};
use std::path::Path;

#[cfg(feature = "zerocopy")]
use zerocopy::{AsBytes, FromBytes, FromZeroes};

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
use crate::mmap::MmappedFile;

/// Zero-copy compatible NBBO record
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "zerocopy", derive(FromZeroes, FromBytes, AsBytes))]
pub struct NbboRecordZC {
    pub ts_event: u64,
    pub ts_recv: u64,
    pub bid_px: i64, // Using i64::MIN as sentinel for None
    pub bid_sz: u32,
    pub bid_ct: u32,
    pub ask_px: i64, // Using i64::MIN as sentinel for None
    pub ask_sz: u32,
    pub ask_ct: u32,
}

impl NbboRecordZC {
    const NONE_SENTINEL: i64 = i64::MIN;

    /// Convert to regular NbboRecord
    pub fn to_record(&self) -> NbboRecord {
        NbboRecord {
            ts_event: self.ts_event,
            ts_recv: self.ts_recv,
            bid_px: if self.bid_px == Self::NONE_SENTINEL {
                None
            } else {
                Some(self.bid_px)
            },
            bid_sz: self.bid_sz,
            bid_ct: self.bid_ct,
            ask_px: if self.ask_px == Self::NONE_SENTINEL {
                None
            } else {
                Some(self.ask_px)
            },
            ask_sz: self.ask_sz,
            ask_ct: self.ask_ct,
        }
    }

    /// Create from regular NbboRecord
    pub fn from_record(record: &NbboRecord) -> Self {
        NbboRecordZC {
            ts_event: record.ts_event,
            ts_recv: record.ts_recv,
            bid_px: record.bid_px.unwrap_or(Self::NONE_SENTINEL),
            bid_sz: record.bid_sz,
            bid_ct: record.bid_ct,
            ask_px: record.ask_px.unwrap_or(Self::NONE_SENTINEL),
            ask_sz: record.ask_sz,
            ask_ct: record.ask_ct,
        }
    }
}

/// Memory-mapped NBBO file reader
#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub struct NbboMmapReader {
    mmap: MmappedFile,
    header: NbboHeader,
    data_offset: usize,
    record_count: u64,
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl NbboMmapReader {
    /// Open an NBBO file with memory mapping
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mmap = MmappedFile::open(path)
            .map_err(|e| NbboError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Read and validate header
        if mmap.len() < 8 {
            return Err(NbboError::InvalidData(
                "File too small for header".to_string(),
            ));
        }

        let magic = &mmap.as_bytes()[0..4];
        if magic != NBBO_MAGIC {
            let mut magic_array = [0u8; 4];
            magic_array.copy_from_slice(magic);
            return Err(NbboError::InvalidMagic(magic_array));
        }

        let version = u16::from_le_bytes([mmap.as_bytes()[4], mmap.as_bytes()[5]]);
        if version != EXPECTED_VERSION {
            return Err(NbboError::InvalidVersion(version));
        }

        let header_length = u16::from_le_bytes([mmap.as_bytes()[6], mmap.as_bytes()[7]]) as usize;

        // Parse full header (simplified for this example)
        let created_at_ns = u64::from_le_bytes(mmap.as_bytes()[8..16].try_into().unwrap());
        let record_count = u64::from_le_bytes(mmap.as_bytes()[16..24].try_into().unwrap());
        let instrument_id = u32::from_le_bytes(mmap.as_bytes()[24..28].try_into().unwrap());

        // Extract symbol (assuming it's at a fixed offset for simplicity)
        let symbol_len = mmap.as_bytes()[28] as usize;
        let symbol = String::from_utf8(mmap.as_bytes()[29..29 + symbol_len].to_vec())?;

        let header = NbboHeader {
            magic: NBBO_MAGIC,
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

        Ok(NbboMmapReader {
            mmap,
            header,
            data_offset: header_length,
            record_count,
        })
    }

    /// Get the header
    pub fn header(&self) -> &NbboHeader {
        &self.header
    }

    /// Get a single record (zero-copy, converted to an owned value)
    pub fn get_record(&self, index: u64) -> Option<NbboRecord> {
        if index >= self.record_count {
            return None;
        }

        let offset = self.data_offset + (index as usize * std::mem::size_of::<NbboRecordZC>());
        self.mmap
            .cast::<NbboRecordZC>(offset)
            .map(|r| r.to_record())
    }

    /// Get a single zero-copy record reference
    pub fn get_record_zc(&self, index: u64) -> Option<&NbboRecordZC> {
        if index >= self.record_count {
            return None;
        }

        let offset = self.data_offset + (index as usize * std::mem::size_of::<NbboRecordZC>());
        self.mmap.cast::<NbboRecordZC>(offset)
    }

    /// Get a slice of records (zero-copy)
    pub fn get_records(&self, start: u64, count: u64) -> Option<Vec<NbboRecord>> {
        if start + count > self.record_count {
            return None;
        }

        let offset = self.data_offset + (start as usize * std::mem::size_of::<NbboRecordZC>());
        self.mmap
            .cast_slice::<NbboRecordZC>(offset, count as usize)
            .map(|slice| slice.iter().map(|r| r.to_record()).collect())
    }

    /// Get a zero-copy slice of records
    pub fn get_records_zc(&self, start: u64, count: u64) -> Option<&[NbboRecordZC]> {
        if start + count > self.record_count {
            return None;
        }

        let offset = self.data_offset + (start as usize * std::mem::size_of::<NbboRecordZC>());
        self.mmap.cast_slice::<NbboRecordZC>(offset, count as usize)
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
    pub fn iter_range(&self, start: u64, end: u64) -> NbboMmapIter<'_> {
        NbboMmapIter {
            reader: self,
            current: start,
            end: end.min(self.record_count),
        }
    }

    /// Create an iterator over all records
    pub fn iter(&self) -> NbboMmapIter<'_> {
        self.iter_range(0, self.record_count)
    }

    /// Get all records as a zero-copy slice (if possible)
    pub fn as_slice(&self) -> Option<&[NbboRecordZC]> {
        self.get_records_zc(0, self.record_count)
    }
}

/// Iterator for memory-mapped NBBO records
#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub struct NbboMmapIter<'a> {
    reader: &'a NbboMmapReader,
    current: u64,
    end: u64,
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl<'a> Iterator for NbboMmapIter<'a> {
    type Item = NbboRecord;

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
impl<'a> ExactSizeIterator for NbboMmapIter<'a> {
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
    fn test_nbbo_record_conversion() {
        let original = NbboRecord {
            ts_event: 1000,
            ts_recv: 1001,
            bid_px: Some(100_000_000_000),
            bid_sz: 100,
            bid_ct: 1,
            ask_px: None,
            ask_sz: 0,
            ask_ct: 0,
        };

        let zc = NbboRecordZC::from_record(&original);
        let converted = zc.to_record();

        assert_eq!(original, converted);
    }

    #[test]
    fn test_nbbo_zero_copy_slice_access() {
        let mut file = NamedTempFile::new().expect("create temp file");

        let mut header = Vec::new();
        header.extend_from_slice(&NBBO_MAGIC);
        header.extend_from_slice(&EXPECTED_VERSION.to_le_bytes());

        let header_length: u16 = 64;
        header.extend_from_slice(&header_length.to_le_bytes());
        header.extend_from_slice(&1_u64.to_le_bytes()); // created_at_ns
        header.extend_from_slice(&2_u64.to_le_bytes()); // record_count
        header.extend_from_slice(&123_u32.to_le_bytes()); // instrument_id
        header.push(4); // symbol length
        header.extend_from_slice(b"TEST");

        while header.len() < header_length as usize {
            header.push(0);
        }

        file.write_all(&header).expect("write header");

        let records = [
            NbboRecord {
                ts_event: 10,
                ts_recv: 11,
                bid_px: Some(100_000_000_000),
                bid_sz: 100,
                bid_ct: 1,
                ask_px: Some(100_500_000_000),
                ask_sz: 120,
                ask_ct: 2,
            },
            NbboRecord {
                ts_event: 20,
                ts_recv: 21,
                bid_px: Some(101_000_000_000),
                bid_sz: 150,
                bid_ct: 1,
                ask_px: Some(101_400_000_000),
                ask_sz: 130,
                ask_ct: 2,
            },
        ];

        for record in &records {
            let zc = NbboRecordZC::from_record(record);
            file.write_all(zc.as_bytes()).expect("write record");
        }

        file.flush().expect("flush temp file");

        let reader = NbboMmapReader::open(file.path()).expect("open mmap reader");

        let slice = reader.get_records_zc(0, 2).expect("zero-copy slice");
        assert_eq!(slice.len(), 2);
        assert_eq!(slice[0].ts_event, 10);
        assert_eq!(slice[1].ts_event, 20);

        let single = reader.get_record_zc(1).expect("record ref");
        assert!(std::ptr::eq(single, &slice[1]));
    }
}
