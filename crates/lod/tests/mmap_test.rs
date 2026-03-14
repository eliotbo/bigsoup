//! Tests for memory-mapped file functionality

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
mod mmap_tests {
    use lod::mmap::{MmappedArray, MmappedFile};
    use std::fs::File;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use zerocopy::{AsBytes, FromBytes, FromZeroes};

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, FromZeroes, FromBytes, AsBytes)]
    struct TestRecord {
        id: u64,
        value: f64,
        flags: u32,
        _padding: u32,
    }

    #[test]
    fn test_mmap_file_basic() {
        let mut temp = NamedTempFile::new().unwrap();
        let data = b"Test data for memory mapping";
        temp.write_all(data).unwrap();
        temp.flush().unwrap();

        let mmap = MmappedFile::open(temp.path()).unwrap();
        assert_eq!(mmap.as_bytes(), data);
        assert_eq!(mmap.len(), data.len());
        assert!(!mmap.is_empty());
    }

    #[test]
    fn test_mmap_typed_array() {
        let mut temp = NamedTempFile::new().unwrap();

        let records = vec![
            TestRecord {
                id: 1,
                value: 1.5,
                flags: 0x01,
                _padding: 0,
            },
            TestRecord {
                id: 2,
                value: 2.5,
                flags: 0x02,
                _padding: 0,
            },
            TestRecord {
                id: 3,
                value: 3.5,
                flags: 0x04,
                _padding: 0,
            },
            TestRecord {
                id: 4,
                value: 4.5,
                flags: 0x08,
                _padding: 0,
            },
            TestRecord {
                id: 5,
                value: 5.5,
                flags: 0x10,
                _padding: 0,
            },
        ];

        for record in &records {
            temp.write_all(record.as_bytes()).unwrap();
        }
        temp.flush().unwrap();

        let array = MmappedArray::<TestRecord>::open(temp.path()).unwrap();
        assert_eq!(array.len(), 5);

        // Test individual access
        for (i, expected) in records.iter().enumerate() {
            let actual = array.get(i).unwrap();
            assert_eq!(actual, expected);
        }

        // Test slice access
        let slice = array.slice(1, 3).unwrap();
        assert_eq!(slice.len(), 3);
        assert_eq!(slice[0], records[1]);
        assert_eq!(slice[1], records[2]);
        assert_eq!(slice[2], records[3]);

        // Test full slice
        let all = array.as_slice().unwrap();
        assert_eq!(all.len(), 5);
        assert_eq!(all, &records[..]);
    }

    #[test]
    fn test_mmap_binary_search() {
        let mut temp = NamedTempFile::new().unwrap();

        let mut records = Vec::new();
        for i in 0..100 {
            records.push(TestRecord {
                id: (i * 10) as u64, // IDs: 0, 10, 20, ..., 990
                value: i as f64,
                flags: 0,
                _padding: 0,
            });
        }

        for record in &records {
            temp.write_all(record.as_bytes()).unwrap();
        }
        temp.flush().unwrap();

        let array = MmappedArray::<TestRecord>::open(temp.path()).unwrap();

        // Test exact match
        let result = array.binary_search_by_key(&500, |r| r.id);
        assert_eq!(result, Ok(50));

        // Test not found (should return insertion point)
        let result = array.binary_search_by_key(&505, |r| r.id);
        assert_eq!(result, Err(51));

        // Test range finding
        let range = array.find_range(200, 400, |r| r.id as i64);
        assert_eq!(range, Some((20, 41)));
    }

    #[test]
    fn test_zero_copy_cast() {
        let mut temp = NamedTempFile::new().unwrap();

        let record = TestRecord {
            id: 0xDEADBEEF,
            value: std::f64::consts::PI,
            flags: 0x12345678,
            _padding: 0,
        };

        temp.write_all(record.as_bytes()).unwrap();
        temp.flush().unwrap();

        let mmap = MmappedFile::open(temp.path()).unwrap();

        // Test single cast
        let casted = mmap.cast::<TestRecord>(0).unwrap();
        assert_eq!(casted.id, 0xDEADBEEF);
        assert_eq!(casted.value, std::f64::consts::PI);
        assert_eq!(casted.flags, 0x12345678);

        // Verify it's truly zero-copy by checking pointer
        let ptr1 = casted as *const TestRecord;
        let ptr2 = mmap.cast::<TestRecord>(0).unwrap() as *const TestRecord;
        assert_eq!(ptr1, ptr2);
    }

    #[test]
    fn test_empty_file_error() {
        let temp = NamedTempFile::new().unwrap();
        let result = MmappedFile::open(temp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_misaligned_file() {
        let mut temp = NamedTempFile::new().unwrap();

        // Write data that's not aligned to TestRecord size
        temp.write_all(&[0u8; 25]).unwrap(); // 25 bytes, not divisible by 24
        temp.flush().unwrap();

        let result = MmappedArray::<TestRecord>::open(temp.path());
        assert!(result.is_err());
    }
}
