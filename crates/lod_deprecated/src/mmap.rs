//! Memory-mapped file support for zero-copy data access
//!
//! This module provides memory-mapped file utilities for efficient, zero-copy
//! access to large binary data files. It enables direct reading from disk-backed
//! memory without loading entire files into RAM.

#[cfg(feature = "mmap")]
use memmap2::{Mmap, MmapOptions};
use std::fs::File;
use std::io;
use std::ops::Deref;
use std::path::Path;

#[cfg(feature = "zerocopy")]
use zerocopy::FromBytes;

/// Error type for memory-mapping operations
#[derive(Debug)]
pub enum MmapError {
    Io(io::Error),
    InvalidSize {
        expected: usize,
        actual: usize,
    },
    AlignmentError,
    #[cfg(feature = "mmap")]
    MmapFailed(String),
}

impl std::fmt::Display for MmapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MmapError::Io(e) => write!(f, "I/O error: {}", e),
            MmapError::InvalidSize { expected, actual } => {
                write!(
                    f,
                    "Invalid file size: expected {}, got {}",
                    expected, actual
                )
            }
            MmapError::AlignmentError => write!(f, "Memory alignment error"),
            #[cfg(feature = "mmap")]
            MmapError::MmapFailed(msg) => write!(f, "Memory mapping failed: {}", msg),
        }
    }
}

impl std::error::Error for MmapError {}

impl From<io::Error> for MmapError {
    fn from(e: io::Error) -> Self {
        MmapError::Io(e)
    }
}

/// Memory-mapped file wrapper for zero-copy access
#[cfg(feature = "mmap")]
pub struct MmappedFile {
    _file: File,
    mmap: Mmap,
}

#[cfg(feature = "mmap")]
impl MmappedFile {
    /// Open and memory-map a file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, MmapError> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;

        if metadata.len() == 0 {
            return Err(MmapError::InvalidSize {
                expected: 1,
                actual: 0,
            });
        }

        let mmap = unsafe {
            MmapOptions::new()
                .map(&file)
                .map_err(|e| MmapError::MmapFailed(e.to_string()))?
        };

        Ok(MmappedFile { _file: file, mmap })
    }

    /// Get the memory-mapped data as a byte slice
    pub fn as_bytes(&self) -> &[u8] {
        &self.mmap
    }

    /// Get the size of the mapped file
    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    /// Check if the file is empty
    pub fn is_empty(&self) -> bool {
        self.mmap.is_empty()
    }

    /// Cast a region to a slice of typed records (zero-copy)
    #[cfg(feature = "zerocopy")]
    pub fn cast_slice<T: FromBytes>(&self, offset: usize, count: usize) -> Option<&[T]> {
        let size = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        let end = offset + (count * size);

        if end > self.mmap.len() {
            return None;
        }

        let bytes = &self.mmap[offset..end];
        let ptr = bytes.as_ptr();

        // Check alignment
        if ptr as usize % align != 0 {
            return None;
        }

        // Use zerocopy for safe conversion when possible
        if let Some(slice_ref) = zerocopy::Ref::<_, [T]>::new_slice(bytes) {
            return Some(slice_ref.into_slice());
        }

        // Fallback to unsafe if zerocopy fails (shouldn't happen with proper alignment)
        unsafe { Some(std::slice::from_raw_parts(ptr as *const T, count)) }
    }

    /// Cast a region to a single typed record (zero-copy)
    #[cfg(feature = "zerocopy")]
    pub fn cast<T: FromBytes>(&self, offset: usize) -> Option<&T> {
        let size = std::mem::size_of::<T>();
        let end = offset + size;

        if end > self.mmap.len() {
            return None;
        }

        let bytes = &self.mmap[offset..end];
        zerocopy::Ref::<_, T>::new(bytes).map(|r| r.into_ref())
    }
}

#[cfg(feature = "mmap")]
impl Deref for MmappedFile {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.mmap
    }
}

/// Memory-mapped array view for typed data
#[cfg(all(feature = "mmap", feature = "zerocopy"))]
pub struct MmappedArray<T: FromBytes> {
    mmap: MmappedFile,
    count: usize,
    _phantom: std::marker::PhantomData<T>,
}

#[cfg(all(feature = "mmap", feature = "zerocopy"))]
impl<T: FromBytes> MmappedArray<T> {
    /// Create a memory-mapped array from a file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, MmapError> {
        let mmap = MmappedFile::open(path)?;
        let size = std::mem::size_of::<T>();

        if mmap.len() % size != 0 {
            return Err(MmapError::AlignmentError);
        }

        let count = mmap.len() / size;

        Ok(MmappedArray {
            mmap,
            count,
            _phantom: std::marker::PhantomData,
        })
    }

    /// Get the number of elements
    pub fn len(&self) -> usize {
        self.count
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Get element at index (zero-copy)
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.count {
            return None;
        }

        let offset = index * std::mem::size_of::<T>();
        self.mmap.cast(offset)
    }

    /// Get a slice of elements (zero-copy)
    pub fn slice(&self, start: usize, count: usize) -> Option<&[T]> {
        if start + count > self.count {
            return None;
        }

        let offset = start * std::mem::size_of::<T>();
        self.mmap.cast_slice(offset, count)
    }

    /// Get all elements as a slice (zero-copy)
    pub fn as_slice(&self) -> Option<&[T]> {
        self.mmap.cast_slice(0, self.count)
    }

    /// Binary search by key
    pub fn binary_search_by_key<B, F>(&self, b: &B, mut f: F) -> Result<usize, usize>
    where
        F: FnMut(&T) -> B,
        B: Ord,
    {
        let mut left = 0;
        let mut right = self.count;

        while left < right {
            let mid = left + (right - left) / 2;

            if let Some(item) = self.get(mid) {
                match f(item).cmp(b) {
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

    /// Find range of elements within time bounds
    pub fn find_range<F>(&self, start_key: i64, end_key: i64, key_fn: F) -> Option<(usize, usize)>
    where
        F: Fn(&T) -> i64 + Copy,
    {
        let start_idx = match self.binary_search_by_key(&start_key, key_fn) {
            Ok(idx) => idx,
            Err(idx) => idx,
        };

        let end_idx = match self.binary_search_by_key(&end_key, key_fn) {
            Ok(idx) => idx + 1,
            Err(idx) => idx,
        };

        if start_idx < end_idx && end_idx <= self.count {
            Some((start_idx, end_idx))
        } else {
            None
        }
    }
}

/// Fallback for non-mmap builds - regular file-backed array
#[cfg(not(feature = "mmap"))]
pub struct MmappedArray<T> {
    data: Vec<T>,
}

#[cfg(not(feature = "mmap"))]
impl<T: Clone> MmappedArray<T> {
    pub fn from_vec(data: Vec<T>) -> Self {
        MmappedArray { data }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.data.get(index)
    }

    pub fn slice(&self, start: usize, count: usize) -> Option<&[T]> {
        let end = start + count;
        if end <= self.data.len() {
            Some(&self.data[start..end])
        } else {
            None
        }
    }

    pub fn as_slice(&self) -> Option<&[T]> {
        Some(&self.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    #[cfg(feature = "mmap")]
    fn test_mmap_basic() {
        let mut temp = NamedTempFile::new().unwrap();
        let data = b"Hello, memory-mapped world!";
        temp.write_all(data).unwrap();
        temp.flush().unwrap();

        let mmap = MmappedFile::open(temp.path()).unwrap();
        assert_eq!(mmap.as_bytes(), data);
        assert_eq!(mmap.len(), data.len());
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq)]
    #[cfg_attr(feature = "zerocopy", derive(FromZeroes, FromBytes, AsBytes))]
    struct TestRecord {
        timestamp: i64,
        value: f64,
    }

    #[test]
    #[cfg(all(feature = "mmap", feature = "zerocopy"))]
    fn test_typed_mmap() {
        let mut temp = NamedTempFile::new().unwrap();

        let records = vec![
            TestRecord {
                timestamp: 100,
                value: 1.5,
            },
            TestRecord {
                timestamp: 200,
                value: 2.5,
            },
            TestRecord {
                timestamp: 300,
                value: 3.5,
            },
        ];

        for record in &records {
            temp.write_all(record.as_bytes()).unwrap();
        }
        temp.flush().unwrap();

        let array = MmappedArray::<TestRecord>::open(temp.path()).unwrap();
        assert_eq!(array.len(), 3);

        assert_eq!(array.get(0), Some(&records[0]));
        assert_eq!(array.get(1), Some(&records[1]));
        assert_eq!(array.get(2), Some(&records[2]));

        let slice = array.slice(1, 2).unwrap();
        assert_eq!(slice.len(), 2);
        assert_eq!(slice[0], records[1]);
    }
}
