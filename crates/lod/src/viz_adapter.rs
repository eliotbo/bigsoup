//! Compatibility adapter for viz crate migration from data to lod
//!
//! This module provides drop-in replacement functions that match the API
//! expected by the viz crate, allowing seamless migration from the data subcrate.

use crate::levels::{LevelStore, PlotCandle};
use crate::nohlcv_decoder::NohlcvReader;
use crate::traits::OhlcvRecord as AggregatorRecord;
use crate::StreamingAggregator;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Data source type for tracking the origin of candle data
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DataSource {
    Csv,
    Nohlcv,
    Nbbo,
}

/// Metadata structure for additional information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub symbol: String,
    pub instrument_id: u32,
    pub trade_count: Option<u32>,
    pub turnover: Option<u64>,
}

/// Candle struct that matches the data subcrate interface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub source: DataSource,
    pub metadata: Option<Metadata>,
}

/// Time struct for compatibility
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Time(pub i64);

impl From<Time> for i64 {
    fn from(t: Time) -> Self {
        t.0
    }
}

impl From<Time> for f64 {
    fn from(t: Time) -> Self {
        t.0 as f64
    }
}

/// OHLC trait for compatibility
pub trait Ohlc {
    fn time(&self) -> Time;
    fn unix_time(&self) -> f64;
    fn open(&self) -> f64;
    fn high(&self) -> f64;
    fn low(&self) -> f64;
    fn close(&self) -> f64;
    fn volume(&self) -> f64;
}

impl Ohlc for Candle {
    fn time(&self) -> Time {
        Time(self.timestamp)
    }
    fn unix_time(&self) -> f64 {
        self.timestamp as f64
    }
    fn open(&self) -> f64 {
        self.open
    }
    fn high(&self) -> f64 {
        self.high
    }
    fn low(&self) -> f64 {
        self.low
    }
    fn close(&self) -> f64 {
        self.close
    }
    fn volume(&self) -> f64 {
        self.volume
    }
}

/// Convert PlotCandle to Candle for backward compatibility
pub fn plot_candle_to_candle(pc: &PlotCandle) -> Candle {
    Candle {
        timestamp: pc.ts / 1_000_000_000, // Convert nanoseconds to seconds
        open: pc.open as f64,
        high: pc.high as f64,
        low: pc.low as f64,
        close: pc.close as f64,
        volume: pc.volume as f64,
        source: DataSource::Nohlcv,
        metadata: None,
    }
}

/// Convert Candle to PlotCandle
pub fn candle_to_plot_candle(candle: &Candle) -> PlotCandle {
    PlotCandle::new(
        candle.timestamp * 1_000_000_000, // Convert seconds to nanoseconds
        candle.open as f32,
        candle.high as f32,
        candle.low as f32,
        candle.close as f32,
        candle.volume as f32,
    )
}

/// Import candles from a CSV file (matches data::import_candles)
pub fn import_candles(
    path: &str,
    range: Option<(i64, i64)>,
) -> Result<Vec<Candle>, Box<dyn std::error::Error>> {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut data = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let parts: Vec<&str> = line.split(',').collect();

        if parts.len() >= 6 {
            let timestamp: i64 = parts[0].parse()?;

            if let Some((start, end)) = range {
                if timestamp < start || timestamp > end {
                    continue;
                }
            }

            let open: f64 = parts[1].parse()?;
            let high: f64 = parts[2].parse()?;
            let low: f64 = parts[3].parse()?;
            let close: f64 = parts[4].parse()?;
            let volume: f64 = parts[5].parse()?;

            data.push(Candle {
                timestamp,
                open,
                high,
                low,
                close,
                volume,
                source: DataSource::Csv,
                metadata: None,
            });
        }
    }

    Ok(data)
}

/// Load NOHLCV data and convert to Candles
pub fn load_nohlcv_data(
    path: &str,
    range: Option<(i64, i64)>,
) -> Result<Vec<Candle>, Box<dyn std::error::Error>> {
    // Use auto-discovery to automatically load splits from splits.json in the same directory
    let mut reader = NohlcvReader::open_with_auto_splits(path)?;
    let mut data = Vec::new();

    // Get symbol from header for metadata
    let symbol = reader.symbol().to_string();
    let instrument_id = reader.header().instrument_id;

    for result in reader.iter() {
        let record = result?;
        let timestamp = (record.ts_event / 1_000_000_000) as i64;

        if let Some((start, end)) = range {
            if timestamp < start {
                continue;
            }
            if timestamp > end {
                break;
            }
        }

        if record.has_valid_prices() {
            data.push(Candle {
                timestamp,
                open: record.open().unwrap_or(0.0),
                high: record.high().unwrap_or(0.0),
                low: record.low().unwrap_or(0.0),
                close: record.close().unwrap_or(0.0),
                volume: record.volume as f64,
                source: DataSource::Nohlcv,
                metadata: Some(Metadata {
                    symbol: symbol.clone(),
                    instrument_id,
                    trade_count: Some(record.trade_count),
                    turnover: Some(record.turnover),
                }),
            });
        }
    }

    Ok(data)
}

/// Detect format and load data accordingly (matches data::load_data_unified)
pub fn load_data_unified(
    path: &str,
    range: Option<(i64, i64)>,
) -> Result<Vec<Candle>, Box<dyn std::error::Error>> {
    let path_obj = Path::new(path);

    if path_obj.extension().and_then(|s| s.to_str()) == Some("csv") {
        import_candles(path, range)
    } else if path_obj.extension().and_then(|s| s.to_str()) == Some("nohlcv") {
        load_nohlcv_data(path, range)
    } else {
        // Try CSV as default
        import_candles(path, range)
    }
}

/// Aggregate candles by interval (matches data::aggregate_by_interval)
pub fn aggregate_by_interval(data: &[Candle], interval_minutes: u64) -> Vec<Candle> {
    if data.is_empty() {
        return Vec::new();
    }

    let interval = interval_minutes * 60;
    let mut out = Vec::new();
    let mut bucket = (data[0].unix_time() as u64) / interval;
    let mut current = data[0].clone();
    current.timestamp = (bucket * interval) as i64;

    for d in &data[1..] {
        let b = (d.unix_time() as u64) / interval;
        if b != bucket {
            out.push(current.clone());
            bucket = b;
            current = d.clone();
            current.timestamp = (bucket * interval) as i64;
        } else {
            if d.high > current.high {
                current.high = d.high;
            }
            if d.low < current.low {
                current.low = d.low;
            }
            current.close = d.close;
            current.volume += d.volume;

            // Aggregate metadata if available
            if let (Some(current_meta), Some(d_meta)) = (&mut current.metadata, &d.metadata) {
                if let (Some(current_trade_count), Some(d_trade_count)) =
                    (current_meta.trade_count, d_meta.trade_count)
                {
                    current_meta.trade_count = Some(current_trade_count + d_trade_count);
                }
                if let (Some(current_turnover), Some(d_turnover)) =
                    (current_meta.turnover, d_meta.turnover)
                {
                    current_meta.turnover = Some(current_turnover + d_turnover);
                }
            }
        }
    }

    out.push(current);
    out
}

/// Aggregate PlotCandles by interval (more efficient version)
pub fn aggregate_plot_candles(data: &[PlotCandle], interval_secs: u64) -> Vec<PlotCandle> {
    if data.is_empty() {
        return Vec::new();
    }

    let interval_ns = interval_secs * 1_000_000_000;
    let mut out = Vec::new();
    let mut bucket = (data[0].ts as u64) / interval_ns;
    let mut current = PlotCandle::new(
        (bucket * interval_ns) as i64,
        data[0].open,
        data[0].high,
        data[0].low,
        data[0].close,
        data[0].volume,
    );

    for d in &data[1..] {
        let b = (d.ts as u64) / interval_ns;
        if b != bucket {
            out.push(current);
            bucket = b;
            current = PlotCandle::new(
                (bucket * interval_ns) as i64,
                d.open,
                d.high,
                d.low,
                d.close,
                d.volume,
            );
        } else {
            if d.high > current.high {
                current.high = d.high;
            }
            if d.low < current.low {
                current.low = d.low;
            }
            current.close = d.close;
            current.volume += d.volume;
        }
    }

    out.push(current);
    out
}

/// Load NOHLCV data and aggregate it into a LevelStore using the LOD pipeline.
///
/// This provides a reusable entry point for visualization crates to obtain
/// multi-resolution candle data without duplicating aggregation logic.
pub fn load_nohlcv_level_store<P: AsRef<Path>>(
    path: P,
    base_interval_secs: u64,
    intervals: &[u64],
) -> Result<LevelStore, Box<dyn std::error::Error + Send + Sync + 'static>> {
    load_nohlcv_level_store_with_days(path, base_interval_secs, intervals, None)
}

/// Load NOHLCV data with optional time filtering (last N days only).
///
/// If `last_n_days` is Some(n), only loads the last n days of data from the file.
/// This significantly reduces memory usage and load time for large datasets.
pub fn load_nohlcv_level_store_with_days<P: AsRef<Path>>(
    path: P,
    base_interval_secs: u64,
    intervals: &[u64],
    last_n_days: Option<i64>,
) -> Result<LevelStore, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let path = path.as_ref();
    // Use auto-discovery to automatically load splits from splits.json in the same directory
    let mut reader = NohlcvReader::open_with_auto_splits(path)?;
    let mut aggregator = StreamingAggregator::new(base_interval_secs, intervals.to_vec());

    // Calculate the cutoff timestamp if filtering by days
    let cutoff_ts = if let Some(days) = last_n_days {
        // Get current time in nanoseconds
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        // Calculate cutoff (days ago in nanoseconds)
        let days_in_ns = (days * 24 * 60 * 60 * 1_000_000_000) as u64;
        Some(now.saturating_sub(days_in_ns))
    } else {
        None
    };

    let mut records_loaded = 0;
    let mut records_skipped = 0;

    for result in reader.iter() {
        let record = result?;

        // Skip records older than cutoff
        if let Some(cutoff) = cutoff_ts {
            if record.ts_event < cutoff {
                records_skipped += 1;
                continue;
            }
        }

        let adapter = AggregatorRecord {
            ts_event: record.ts_event,
            open_px: record.open_px,
            high_px: record.high_px,
            low_px: record.low_px,
            close_px: record.close_px,
            volume: record.volume,
            trade_count: record.trade_count,
        };
        aggregator.push(&adapter);
        records_loaded += 1;
    }

    if let Some(days) = last_n_days {
        println!(
            "Loaded {} records from last {} days (skipped {} older records)",
            records_loaded, days, records_skipped
        );
    }

    let batch = aggregator.seal();
    let levels: Vec<(u64, Vec<PlotCandle>)> = batch.levels.into_iter().collect();
    Ok(LevelStore::from_stream(levels))
}

/// Format timestamp (matches data::format_timestamp)
pub fn format_timestamp(value: f64, step: f64) -> String {
    use chrono::{DateTime, Utc};
    use chrono_tz::America::New_York;

    let secs = value as i64;
    let dt_utc = DateTime::<Utc>::from_timestamp(secs, 0).unwrap();
    let dt_et = dt_utc.with_timezone(&New_York);

    if step >= 86_400.0 {
        dt_et.format("%Y-%m-%d").to_string()
    } else {
        dt_et.format("%m-%d %H:%M %Z").to_string()
    }
}

/// View utilities re-exports
pub mod view_utils {
    /// Calculate a nice step size for axis labels
    pub fn nice_step(range: f64, target_steps: usize) -> f64 {
        let rough_step = range / target_steps as f64;
        let magnitude = 10_f64.powf(rough_step.log10().floor());
        let normalized = rough_step / magnitude;

        let nice = if normalized <= 1.0 {
            1.0
        } else if normalized <= 2.0 {
            2.0
        } else if normalized <= 5.0 {
            5.0
        } else {
            10.0
        };

        nice * magnitude
    }

    /// Slice data centered around a value
    pub fn slice_centered<T: Clone>(data: &[T], center: usize, radius: usize) -> Vec<T> {
        let start = center.saturating_sub(radius);
        let end = (center + radius + 1).min(data.len());
        data[start..end].to_vec()
    }

    /// Slice data centered around a time value
    pub fn slice_centered_time<T>(
        data: &[T],
        timestamps: &[i64],
        center_time: i64,
        radius_time: i64,
    ) -> Vec<T>
    where
        T: Clone,
    {
        let mut start = 0;
        let mut end = data.len();

        for (i, &ts) in timestamps.iter().enumerate() {
            if ts < center_time - radius_time && i + 1 < timestamps.len() {
                start = i + 1;
            }
            if ts > center_time + radius_time {
                end = i;
                break;
            }
        }

        data[start..end].to_vec()
    }

    /// Calculate visible rows for display
    pub fn visible_rows(total: usize, viewport_height: f64, row_height: f64) -> usize {
        ((viewport_height / row_height).ceil() as usize).min(total)
    }
}

/// Trading time utilities
pub mod trading_time {
    use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Timelike, Utc, Weekday};
    use chrono_tz::America::New_York;
    use std::collections::HashSet;

    /// Market hours definition
    pub struct MarketHours {
        pub open_hour: u32,
        pub open_minute: u32,
        pub close_hour: u32,
        pub close_minute: u32,
    }

    impl Default for MarketHours {
        fn default() -> Self {
            MarketHours {
                open_hour: 9,
                open_minute: 30,
                close_hour: 16,
                close_minute: 0,
            }
        }
    }

    /// Check if a given day is a trading day
    pub fn is_trading_day(date: NaiveDate) -> bool {
        // Skip weekends
        match date.weekday() {
            Weekday::Sat | Weekday::Sun => return false,
            _ => {}
        }

        // Check for US market holidays (simplified list)
        let holidays = get_us_market_holidays(date.year());
        !holidays.contains(&date)
    }

    /// Check if a timestamp falls within trading hours
    pub fn is_trading_hours(timestamp: i64, hours: &MarketHours) -> bool {
        let dt = DateTime::<Utc>::from_timestamp(timestamp, 0).unwrap();
        let dt_et = dt.with_timezone(&New_York);

        if !is_trading_day(dt_et.date_naive()) {
            return false;
        }

        let hour = dt_et.hour();
        let minute = dt_et.minute();

        let open_minutes = hours.open_hour * 60 + hours.open_minute;
        let close_minutes = hours.close_hour * 60 + hours.close_minute;
        let current_minutes = hour * 60 + minute;

        current_minutes >= open_minutes && current_minutes < close_minutes
    }

    fn get_us_market_holidays(year: i32) -> HashSet<NaiveDate> {
        let mut holidays = HashSet::new();

        // New Year's Day
        holidays.insert(NaiveDate::from_ymd_opt(year, 1, 1).unwrap());

        // Martin Luther King Jr. Day (3rd Monday in January)
        // Presidents' Day (3rd Monday in February)
        // Good Friday (varies)
        // Memorial Day (last Monday in May)
        // Independence Day
        holidays.insert(NaiveDate::from_ymd_opt(year, 7, 4).unwrap());
        // Labor Day (1st Monday in September)
        // Thanksgiving (4th Thursday in November)
        // Christmas
        holidays.insert(NaiveDate::from_ymd_opt(year, 12, 25).unwrap());

        holidays
    }

    /// Trading day mapper for compressed time axis
    pub struct TradingDayMapper {
        trading_days: Vec<i64>,
        day_to_index: HashMap<i64, usize>,
    }

    use std::collections::HashMap;

    impl TradingDayMapper {
        pub fn new(start: i64, end: i64) -> Self {
            let mut trading_days = Vec::new();
            let mut day_to_index = HashMap::new();

            let mut current = start;
            let mut index = 0;

            while current <= end {
                let dt = DateTime::<Utc>::from_timestamp(current, 0).unwrap();
                if is_trading_day(dt.date_naive()) {
                    trading_days.push(current);
                    day_to_index.insert(current, index);
                    index += 1;
                }
                current += 86400; // Add one day
            }

            TradingDayMapper {
                trading_days,
                day_to_index,
            }
        }

        pub fn compress_coordinate(&self, timestamp: i64) -> f64 {
            // Find the trading day
            let day_start = (timestamp / 86400) * 86400;

            if let Some(&index) = self.day_to_index.get(&day_start) {
                // Add intraday fraction
                let fraction = (timestamp - day_start) as f64 / 86400.0;
                index as f64 + fraction
            } else {
                // Estimate position for non-trading days
                timestamp as f64 / 86400.0
            }
        }

        pub fn expand_coordinate(&self, compressed: f64) -> i64 {
            let day_index = compressed.floor() as usize;
            let fraction = compressed.fract();

            if day_index < self.trading_days.len() {
                let day_start = self.trading_days[day_index];
                day_start + (fraction * 86400.0) as i64
            } else {
                // Fallback for out-of-range
                (compressed * 86400.0) as i64
            }
        }
    }

    /// Trading hours mapper for intraday compression
    pub struct TradingHoursMapper {
        hours: MarketHours,
        session_duration: i64,
    }

    impl TradingHoursMapper {
        pub fn new(hours: MarketHours) -> Self {
            let session_duration = ((hours.close_hour * 60 + hours.close_minute)
                - (hours.open_hour * 60 + hours.open_minute))
                as i64
                * 60;

            TradingHoursMapper {
                hours,
                session_duration,
            }
        }

        pub fn compress_intraday(&self, timestamp: i64) -> Option<f64> {
            if !is_trading_hours(timestamp, &self.hours) {
                return None;
            }

            let dt = DateTime::<Utc>::from_timestamp(timestamp, 0).unwrap();
            let dt_et = dt.with_timezone(&New_York);

            let day_start = dt_et
                .date_naive()
                .and_hms_opt(self.hours.open_hour, self.hours.open_minute, 0)
                .unwrap();

            let day_start_ts = New_York
                .from_local_datetime(&day_start)
                .single()
                .unwrap()
                .timestamp();

            let offset = timestamp - day_start_ts;
            Some(offset as f64 / self.session_duration as f64)
        }
    }
}

/// Data format detection
pub mod format_detection {
    use std::fs::File;
    use std::io::Read;
    use std::path::Path;

    #[derive(Debug, PartialEq)]
    pub enum DataFormat {
        Csv,
        Nohlcv,
    }

    pub fn detect_format(path: &str) -> Result<DataFormat, Box<dyn std::error::Error>> {
        let path_obj = Path::new(path);

        // Check extension first
        if let Some(ext) = path_obj.extension().and_then(|s| s.to_str()) {
            match ext {
                "csv" => return Ok(DataFormat::Csv),
                "nohlcv" => return Ok(DataFormat::Nohlcv),
                _ => {}
            }
        }

        // Check file content
        let mut file = File::open(path)?;
        let mut buffer = [0u8; 4];
        file.read_exact(&mut buffer)?;

        if &buffer == b"NOHL" {
            Ok(DataFormat::Nohlcv)
        } else {
            // Assume CSV if not binary format
            Ok(DataFormat::Csv)
        }
    }
}
