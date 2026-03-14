//! Data loading infrastructure for file discovery and management
//!
//! This module provides high-level data loading capabilities including:
//! - File discovery in data directories
//! - Symbol registry management
//! - Multi-format data loading (NBBO, NOHLCV, CSV)
//! - Date range and interval management

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::levels::{LevelStore, PlotCandle};
use crate::nbbo_decoder::NbboReader;
use crate::nohlcv_decoder::NohlcvReader;

/// Type of data file
#[derive(Debug, Clone, PartialEq)]
pub enum FileType {
    Nbbo,
    Nohlcv,
    Csv,
}

/// Information about a single data file
#[derive(Debug, Clone)]
pub struct DataFile {
    pub path: PathBuf,
    pub file_type: FileType,
    pub interval: Option<u64>, // Seconds (e.g., 60 for 1m)
    pub record_count: u64,
    pub start_date: i64, // Unix timestamp
    pub end_date: i64,   // Unix timestamp
}

/// Metadata for a trading symbol
#[derive(Debug, Clone)]
pub struct SymbolMetadata {
    pub symbol: String,
    pub exchanges: Vec<String>,
    pub date_range: (i64, i64), // Unix timestamps (start, end)
    pub available_files: Vec<DataFile>,
}

/// Main data loader for discovering and loading market data
pub struct DataLoader {
    base_paths: Vec<PathBuf>,
    cache: HashMap<String, SymbolMetadata>,
}

impl DataLoader {
    /// Create a new DataLoader with specified search directories
    pub fn new(base_paths: Vec<PathBuf>) -> Self {
        DataLoader {
            base_paths,
            cache: HashMap::new(),
        }
    }

    /// Create a DataLoader with default paths
    pub fn with_default_paths() -> Self {
        let mut paths = vec![];

        // Check common data locations
        let candidates = vec![
            PathBuf::from("/media/data10t/databento"),
            PathBuf::from("data/consolidated"),
            PathBuf::from("/workspace/workspace/bb/data/consolidated"),
            PathBuf::from("/workspace/workspace/bb/beta_breaker/data/consolidated"),
        ];

        for path in candidates {
            if path.exists() {
                paths.push(path);
            }
        }

        DataLoader::new(paths)
    }

    /// Scan directories for data files and build symbol registry
    pub fn scan_directories(&mut self) -> io::Result<()> {
        self.cache.clear();

        for base_path in &self.base_paths {
            if !base_path.exists() {
                continue;
            }

            // Look for exchange directories (e.g., hedgy-test)
            for exchange_entry in fs::read_dir(base_path)? {
                let exchange_path = exchange_entry?.path();
                if !exchange_path.is_dir() {
                    continue;
                }

                let exchange_name = exchange_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                // Look for symbol directories (e.g., AAPL, CIEN)
                for symbol_entry in fs::read_dir(&exchange_path)? {
                    let symbol_path = symbol_entry?.path();
                    if !symbol_path.is_dir() {
                        continue;
                    }

                    let symbol = symbol_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    if symbol.is_empty() {
                        continue;
                    }

                    // Scan for data files in symbol directory
                    let files = self.scan_symbol_directory(&symbol_path)?;

                    if !files.is_empty() {
                        let date_range = self.calculate_date_range(&files);

                        let metadata =
                            self.cache
                                .entry(symbol.clone())
                                .or_insert_with(|| SymbolMetadata {
                                    symbol: symbol.clone(),
                                    exchanges: vec![],
                                    date_range,
                                    available_files: vec![],
                                });

                        if !metadata.exchanges.contains(&exchange_name) {
                            metadata.exchanges.push(exchange_name.clone());
                        }

                        metadata.available_files.extend(files);

                        // Update date range
                        if date_range.0 < metadata.date_range.0 {
                            metadata.date_range.0 = date_range.0;
                        }
                        if date_range.1 > metadata.date_range.1 {
                            metadata.date_range.1 = date_range.1;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Scan a symbol directory for data files
    fn scan_symbol_directory(&self, path: &Path) -> io::Result<Vec<DataFile>> {
        let mut files = vec![];

        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let file_path = entry.path();

            if !file_path.is_file() {
                continue;
            }

            let filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if let Some(data_file) = self.parse_data_file(&file_path, filename) {
                files.push(data_file);
            }
        }

        Ok(files)
    }

    /// Parse a filename to extract metadata
    fn parse_data_file(&self, path: &Path, filename: &str) -> Option<DataFile> {
        // Parse NBBO files: YYYY-MM-DD_to_YYYY-MM-DD-nbbo-{interval}.nbbo
        if filename.ends_with(".nbbo") {
            if let Some(info) = self.parse_nbbo_filename(filename) {
                let (start_date, end_date, interval) = info;

                // Try to get record count from file
                let record_count = NbboReader::open(path)
                    .ok()
                    .map(|r| r.record_count())
                    .unwrap_or(0);

                return Some(DataFile {
                    path: path.to_path_buf(),
                    file_type: FileType::Nbbo,
                    interval: Some(interval),
                    record_count,
                    start_date,
                    end_date,
                });
            }
        }

        // Parse NOHLCV files: YYYY-MM-DD_to_YYYY-MM-DD-ohlcv-{interval}.nohlcv
        if filename.ends_with(".nohlcv") {
            if let Some(info) = self.parse_nohlcv_filename(filename) {
                let (start_date, end_date, interval) = info;

                // Try to get record count from file
                let record_count = NohlcvReader::open(path)
                    .ok()
                    .map(|r| r.record_count())
                    .unwrap_or(0);

                return Some(DataFile {
                    path: path.to_path_buf(),
                    file_type: FileType::Nohlcv,
                    interval: Some(interval),
                    record_count,
                    start_date,
                    end_date,
                });
            }
        }

        // Parse CSV files (simple numeric names or dates)
        if filename.ends_with(".csv") {
            // For now, use file metadata for dates
            let metadata = fs::metadata(path).ok()?;
            let modified = metadata.modified().ok()?;
            let timestamp = modified
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs() as i64;

            return Some(DataFile {
                path: path.to_path_buf(),
                file_type: FileType::Csv,
                interval: None,
                record_count: 0, // Would need to read file to count
                start_date: timestamp,
                end_date: timestamp,
            });
        }

        None
    }

    /// Parse NBBO filename format
    fn parse_nbbo_filename(&self, filename: &str) -> Option<(i64, i64, u64)> {
        // Format: YYYY-MM-DD_to_YYYY-MM-DD-nbbo-{interval}.nbbo
        let parts: Vec<&str> = filename.strip_suffix(".nbbo")?.split('-').collect();

        if parts.len() >= 6 {
            // Extract dates
            let start_date = self.parse_date(&format!("{}-{}-{}", parts[0], parts[1], parts[2]))?;
            // parts[3] is "to"
            let end_date = self.parse_date(&format!(
                "{}-{}-{}",
                parts[3].strip_prefix("to_")?,
                parts[4],
                parts[5]
            ))?;

            // Extract interval (e.g., "1m" -> 60 seconds)
            let interval_str = parts.get(7)?;
            let interval = self.parse_interval(interval_str)?;

            return Some((start_date, end_date, interval));
        }

        None
    }

    /// Parse NOHLCV filename format
    fn parse_nohlcv_filename(&self, filename: &str) -> Option<(i64, i64, u64)> {
        // Format: YYYY-MM-DD_to_YYYY-MM-DD-ohlcv-{interval}.nohlcv
        let parts: Vec<&str> = filename.strip_suffix(".nohlcv")?.split('-').collect();

        if parts.len() >= 6 {
            // Extract dates
            let start_date = self.parse_date(&format!("{}-{}-{}", parts[0], parts[1], parts[2]))?;
            // parts[3] contains "_to_"
            let end_date = self.parse_date(&format!(
                "{}-{}-{}",
                parts[3].strip_prefix("to_")?,
                parts[4],
                parts[5]
            ))?;

            // Extract interval
            let interval_str = parts.get(7)?;
            let interval = self.parse_interval(interval_str)?;

            return Some((start_date, end_date, interval));
        }

        None
    }

    /// Parse date string to Unix timestamp
    fn parse_date(&self, date_str: &str) -> Option<i64> {
        // Parse YYYY-MM-DD format
        let parts: Vec<&str> = date_str.split('-').collect();
        if parts.len() != 3 {
            return None;
        }

        let year: i32 = parts[0].parse().ok()?;
        let month: u32 = parts[1].parse().ok()?;
        let day: u32 = parts[2].parse().ok()?;

        // Convert to Unix timestamp (simplified)
        use chrono::{TimeZone, Utc};
        let dt = Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).single()?;
        Some(dt.timestamp())
    }

    /// Parse interval string to seconds
    fn parse_interval(&self, interval_str: &str) -> Option<u64> {
        match interval_str {
            "1m" => Some(60),
            "5m" => Some(300),
            "15m" => Some(900),
            "30m" => Some(1800),
            "1h" => Some(3600),
            "4h" => Some(14400),
            "1d" => Some(86400),
            _ => {
                // Try to parse numeric with suffix
                if let Some(num_str) = interval_str.strip_suffix('m') {
                    let minutes: u64 = num_str.parse().ok()?;
                    return Some(minutes * 60);
                }
                if let Some(num_str) = interval_str.strip_suffix('h') {
                    let hours: u64 = num_str.parse().ok()?;
                    return Some(hours * 3600);
                }
                if let Some(num_str) = interval_str.strip_suffix('d') {
                    let days: u64 = num_str.parse().ok()?;
                    return Some(days * 86400);
                }
                None
            }
        }
    }

    /// Calculate date range from a list of files
    fn calculate_date_range(&self, files: &[DataFile]) -> (i64, i64) {
        let mut min_date = i64::MAX;
        let mut max_date = i64::MIN;

        for file in files {
            if file.start_date < min_date {
                min_date = file.start_date;
            }
            if file.end_date > max_date {
                max_date = file.end_date;
            }
        }

        (min_date, max_date)
    }

    /// Get all available symbols
    pub fn get_available_symbols(&self) -> Vec<String> {
        let mut symbols: Vec<String> = self.cache.keys().cloned().collect();
        symbols.sort();
        symbols
    }

    /// Get metadata for a specific symbol
    pub fn get_symbol_info(&self, symbol: &str) -> Option<&SymbolMetadata> {
        self.cache.get(symbol)
    }

    /// Load data for a symbol within a date range
    pub fn load_symbol(
        &mut self,
        symbol: &str,
        start_date: Option<i64>,
        end_date: Option<i64>,
        intervals: &[u64],
    ) -> Result<LevelStore, Box<dyn std::error::Error>> {
        // Ensure cache is populated
        if self.cache.is_empty() {
            self.scan_directories()?;
        }

        let metadata = self
            .cache
            .get(symbol)
            .ok_or_else(|| format!("Symbol {} not found", symbol))?;

        let mut store = LevelStore::new();

        // Find matching files for requested intervals
        for &interval in intervals {
            for file in &metadata.available_files {
                if let Some(file_interval) = file.interval {
                    if file_interval == interval {
                        // Check date range
                        let load = match (start_date, end_date) {
                            (Some(s), Some(e)) => file.start_date <= e && file.end_date >= s,
                            (Some(s), None) => file.end_date >= s,
                            (None, Some(e)) => file.start_date <= e,
                            (None, None) => true,
                        };

                        if load {
                            self.load_file_into_store(file, &mut store, start_date, end_date)?;
                        }
                    }
                }
            }
        }

        Ok(store)
    }

    /// Load a single file into a LevelStore
    fn load_file_into_store(
        &self,
        file: &DataFile,
        store: &mut LevelStore,
        start_date: Option<i64>,
        end_date: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match file.file_type {
            FileType::Nohlcv => {
                let mut reader = NohlcvReader::open(&file.path)?;
                let mut candles = vec![];

                for result in reader.iter() {
                    let record = result?;
                    let timestamp = (record.ts_event / 1_000_000_000) as i64;

                    // Filter by date range
                    if let Some(start) = start_date {
                        if timestamp < start {
                            continue;
                        }
                    }
                    if let Some(end) = end_date {
                        if timestamp > end {
                            break;
                        }
                    }

                    if record.has_valid_prices() {
                        let candle = PlotCandle::new(
                            record.ts_event as i64,
                            record.open().unwrap_or(0.0) as f32,
                            record.high().unwrap_or(0.0) as f32,
                            record.low().unwrap_or(0.0) as f32,
                            record.close().unwrap_or(0.0) as f32,
                            record.volume as f32,
                        );
                        candles.push(candle);
                    }
                }

                if !candles.is_empty() {
                    store.append(file.interval.unwrap_or(60), &candles, false);
                }
            }
            FileType::Nbbo => {
                // NBBO data would need special handling to convert to candles
                // For now, skip NBBO files
            }
            FileType::Csv => {
                // Load CSV data
                let candles = self.load_csv_candles(&file.path, start_date, end_date)?;
                if !candles.is_empty() {
                    store.append(60, &candles, false); // Default to 1m interval for CSV
                }
            }
        }

        Ok(())
    }

    /// Load candles from CSV file
    fn load_csv_candles(
        &self,
        path: &Path,
        start_date: Option<i64>,
        end_date: Option<i64>,
    ) -> Result<Vec<PlotCandle>, Box<dyn std::error::Error>> {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut candles = vec![];

        for line in reader.lines() {
            let line = line?;
            let parts: Vec<&str> = line.split(',').collect();

            if parts.len() >= 6 {
                let timestamp: i64 = parts[0].parse()?;

                // Filter by date range
                if let Some(start) = start_date {
                    if timestamp < start {
                        continue;
                    }
                }
                if let Some(end) = end_date {
                    if timestamp > end {
                        break;
                    }
                }

                let open: f64 = parts[1].parse()?;
                let high: f64 = parts[2].parse()?;
                let low: f64 = parts[3].parse()?;
                let close: f64 = parts[4].parse()?;
                let volume: f64 = parts[5].parse()?;

                let candle = PlotCandle::new(
                    timestamp * 1_000_000_000, // Convert to nanoseconds
                    open as f32,
                    high as f32,
                    low as f32,
                    close as f32,
                    volume as f32,
                );
                candles.push(candle);
            }
        }

        Ok(candles)
    }

    /// Load the most recent data for a symbol
    pub fn load_latest(
        &mut self,
        symbol: &str,
        _count: usize,
        intervals: &[u64],
    ) -> Result<LevelStore, Box<dyn std::error::Error>> {
        // For simplicity, load all data and then trim
        // In production, this would be optimized to read only needed records
        let store = self.load_symbol(symbol, None, None, intervals)?;

        // TODO: Implement trimming to last 'count' records per interval

        Ok(store)
    }
}

/// Load candles from a file path (convenience function)
pub fn load_candles_from_path(path: &str) -> Result<Vec<PlotCandle>, Box<dyn std::error::Error>> {
    let path = Path::new(path);

    if path.extension().and_then(|s| s.to_str()) == Some("nohlcv") {
        let mut reader = NohlcvReader::open(path)?;
        let mut candles = vec![];

        for result in reader.iter() {
            let record = result?;
            if record.has_valid_prices() {
                let candle = PlotCandle::new(
                    record.ts_event as i64,
                    record.open().unwrap_or(0.0) as f32,
                    record.high().unwrap_or(0.0) as f32,
                    record.low().unwrap_or(0.0) as f32,
                    record.close().unwrap_or(0.0) as f32,
                    record.volume as f32,
                );
                candles.push(candle);
            }
        }

        Ok(candles)
    } else if path.extension().and_then(|s| s.to_str()) == Some("csv") {
        let loader = DataLoader::new(vec![]);
        loader.load_csv_candles(path, None, None)
    } else {
        Err("Unsupported file format".into())
    }
}
