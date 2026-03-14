//! Multi-resolution line graph LOD system for efficient large-scale data visualization
//!
//! This module implements Phase 3 of the Line Graph Optimization Plan.
//! It provides a multi-resolution time series structure that automatically
//! selects the appropriate level of detail based on viewport and target points.

use std::collections::BTreeMap;

/// Aggregated point containing OHLC data for a time interval
#[derive(Debug, Clone, Copy)]
pub struct AggregatedPoint {
    pub timestamp_ns: u64,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    pub typical: f32, // (high + low + close) / 3
    pub count: u32,   // Number of points aggregated
}

impl AggregatedPoint {
    /// Create a new aggregated point from a single value
    pub fn from_single(timestamp_ns: u64, value: f32) -> Self {
        Self {
            timestamp_ns,
            open: value,
            high: value,
            low: value,
            close: value,
            typical: value,
            count: 1,
        }
    }

    /// Create an aggregated point from OHLC values
    pub fn from_ohlc(timestamp_ns: u64, open: f32, high: f32, low: f32, close: f32) -> Self {
        let typical = (high + low + close) / 3.0;
        Self {
            timestamp_ns,
            open,
            high,
            low,
            close,
            typical,
            count: 1,
        }
    }

    /// Merge another point into this one (for aggregation)
    pub fn merge(&mut self, other: &AggregatedPoint) {
        if self.count == 0 {
            *self = *other;
            return;
        }

        // Update OHLC values
        self.high = self.high.max(other.high);
        self.low = self.low.min(other.low);
        self.close = other.close; // Last close

        // Update typical as weighted average
        let total_count = self.count + other.count;
        self.typical = (self.typical * self.count as f32 + other.typical * other.count as f32)
            / total_count as f32;
        self.count = total_count;
    }
}

/// Resolution level containing aggregated points at a specific time interval
#[derive(Debug, Clone)]
pub struct ResolutionLevel {
    pub interval_ns: u64,
    pub points: Vec<AggregatedPoint>,
    pub min_zoom: f32,
    pub max_zoom: f32,
    pub min_value: f32,
    pub max_value: f32,
}

impl ResolutionLevel {
    /// Create a new resolution level
    pub fn new(interval_ns: u64, min_zoom: f32, max_zoom: f32) -> Self {
        Self {
            interval_ns,
            points: Vec::new(),
            min_zoom,
            max_zoom,
            min_value: f32::INFINITY,
            max_value: f32::NEG_INFINITY,
        }
    }

    /// Add a pre-aggregated point to this level
    pub fn add_point(&mut self, point: AggregatedPoint) {
        self.min_value = self.min_value.min(point.low);
        self.max_value = self.max_value.max(point.high);
        self.points.push(point);
    }

    /// Get the number of points in this level
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Check if the level is empty
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Find points within a time window using binary search
    pub fn find_window_indices(&self, start_ns: u64, end_ns: u64) -> (usize, usize) {
        let start_idx = self.points.partition_point(|p| p.timestamp_ns < start_ns);
        let end_idx = self.points.partition_point(|p| p.timestamp_ns <= end_ns);
        (start_idx, end_idx)
    }

    /// Get points within a time window
    pub fn get_window_points(&self, start_ns: u64, end_ns: u64) -> &[AggregatedPoint] {
        let (start, end) = self.find_window_indices(start_ns, end_ns);
        &self.points[start..end]
    }
}

/// Multi-resolution time series with automatic LOD selection
#[derive(Debug, Clone)]
pub struct MultiResolutionSeries {
    pub symbol: String,
    pub levels: Vec<ResolutionLevel>,
    min_pixels_per_point: f32,
}

impl MultiResolutionSeries {
    /// Create a new multi-resolution series
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            levels: Vec::new(),
            min_pixels_per_point: 2.0, // Minimum pixels between points for clarity
        }
    }

    /// Set the minimum pixels per point for LOD selection
    pub fn set_min_pixels_per_point(&mut self, min_pixels: f32) {
        self.min_pixels_per_point = min_pixels.max(1.0);
    }

    /// Add a resolution level
    pub fn add_level(&mut self, level: ResolutionLevel) {
        // Insert in order of interval size (finest to coarsest)
        let pos = self
            .levels
            .partition_point(|l| l.interval_ns < level.interval_ns);
        self.levels.insert(pos, level);
    }

    /// Select the best resolution level for a given viewport
    pub fn select_lod(
        &self,
        viewport_start_ns: u64,
        viewport_end_ns: u64,
        viewport_width_px: f32,
    ) -> Option<&ResolutionLevel> {
        if self.levels.is_empty() {
            return None;
        }

        let duration_ns = viewport_end_ns.saturating_sub(viewport_start_ns);
        if duration_ns == 0 {
            return self.levels.first();
        }

        // Calculate target points based on viewport width
        let target_points = (viewport_width_px / self.min_pixels_per_point) as u64;

        // Find the coarsest level that gives us enough resolution
        // Start from coarsest and work towards finest
        for level in self.levels.iter().rev() {
            let estimated_points = duration_ns / level.interval_ns;

            // Check if this level provides enough detail
            if estimated_points >= target_points / 2 && estimated_points <= target_points * 2 {
                return Some(level);
            }

            // If we have too many points at this level, need a coarser one
            if estimated_points > target_points * 2 {
                continue;
            }

            // If we have too few points, this is the best we can do
            if estimated_points < target_points / 2 {
                return Some(level);
            }
        }

        // Default to finest level
        self.levels.first()
    }

    /// Get value range across all levels
    pub fn get_value_range(&self) -> Option<(f32, f32)> {
        if self.levels.is_empty() {
            return None;
        }

        let min = self
            .levels
            .iter()
            .map(|l| l.min_value)
            .fold(f32::INFINITY, f32::min);
        let max = self
            .levels
            .iter()
            .map(|l| l.max_value)
            .fold(f32::NEG_INFINITY, f32::max);

        Some((min, max))
    }

    /// Create a new resolution level by downsampling from raw data
    pub fn create_level_from_raw(
        &mut self,
        raw_points: &[(u64, f32)],
        interval_ns: u64,
        min_zoom: f32,
        max_zoom: f32,
    ) {
        let mut level = ResolutionLevel::new(interval_ns, min_zoom, max_zoom);

        if raw_points.is_empty() {
            self.add_level(level);
            return;
        }

        // Group points by interval buckets
        let mut buckets: BTreeMap<u64, Vec<f32>> = BTreeMap::new();

        for &(timestamp_ns, value) in raw_points {
            let bucket = (timestamp_ns / interval_ns) * interval_ns;
            buckets.entry(bucket).or_insert_with(Vec::new).push(value);
        }

        // Create aggregated points from buckets
        for (bucket_time, values) in buckets {
            if values.is_empty() {
                continue;
            }

            let open = values[0];
            let close = values[values.len() - 1];
            let high = values.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            let low = values.iter().fold(f32::INFINITY, |a, &b| a.min(b));

            let point = AggregatedPoint::from_ohlc(bucket_time, open, high, low, close);
            level.add_point(point);
        }

        self.add_level(level);
    }

    /// Build standard resolution levels from raw tick data
    pub fn build_standard_levels(&mut self, raw_points: &[(u64, f32)]) {
        // Define standard intervals with zoom ranges
        let intervals = [
            (1_000_000_000, 0.0, 1.0),                  // 1 second (max zoom)
            (10_000_000_000, 1.0, 5.0),                 // 10 seconds
            (60_000_000_000, 5.0, 30.0),                // 1 minute
            (300_000_000_000, 30.0, 120.0),             // 5 minutes
            (900_000_000_000, 120.0, 360.0),            // 15 minutes
            (3600_000_000_000, 360.0, 1440.0),          // 1 hour
            (86400_000_000_000, 1440.0, f32::INFINITY), // 1 day
        ];

        for (interval_ns, min_zoom, max_zoom) in intervals {
            self.create_level_from_raw(raw_points, interval_ns, min_zoom, max_zoom);
        }
    }
}

/// Progressive loader for background LOD generation
pub struct ProgressiveLodLoader {
    series: MultiResolutionSeries,
    pending_levels: Vec<(u64, f32, f32)>, // (interval_ns, min_zoom, max_zoom)
    raw_data: Vec<(u64, f32)>,
}

impl ProgressiveLodLoader {
    /// Create a new progressive loader
    pub fn new(symbol: String) -> Self {
        Self {
            series: MultiResolutionSeries::new(symbol),
            pending_levels: Vec::new(),
            raw_data: Vec::new(),
        }
    }

    /// Set the raw data for LOD generation
    pub fn set_raw_data(&mut self, data: Vec<(u64, f32)>) {
        self.raw_data = data;

        // Queue standard resolution levels
        self.pending_levels = vec![
            (86400_000_000_000, 1440.0, f32::INFINITY), // Start with coarsest (1 day)
            (3600_000_000_000, 360.0, 1440.0),          // 1 hour
            (900_000_000_000, 120.0, 360.0),            // 15 minutes
            (300_000_000_000, 30.0, 120.0),             // 5 minutes
            (60_000_000_000, 5.0, 30.0),                // 1 minute
            (10_000_000_000, 1.0, 5.0),                 // 10 seconds
            (1_000_000_000, 0.0, 1.0),                  // 1 second (finest)
        ];
    }

    /// Process the next pending level (for progressive loading)
    pub fn process_next_level(&mut self) -> bool {
        if let Some((interval_ns, min_zoom, max_zoom)) = self.pending_levels.pop() {
            self.series
                .create_level_from_raw(&self.raw_data, interval_ns, min_zoom, max_zoom);
            true
        } else {
            false
        }
    }

    /// Get the current series (may be partially loaded)
    pub fn get_series(&self) -> &MultiResolutionSeries {
        &self.series
    }

    /// Check if all levels are loaded
    pub fn is_complete(&self) -> bool {
        self.pending_levels.is_empty()
    }

    /// Get loading progress as a percentage
    pub fn get_progress(&self) -> f32 {
        let total = 7.0; // Total standard levels
        let loaded = total - self.pending_levels.len() as f32;
        (loaded / total) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregated_point_merge() {
        let mut point1 = AggregatedPoint::from_single(1000, 100.0);
        let point2 = AggregatedPoint::from_single(2000, 110.0);

        point1.merge(&point2);

        assert_eq!(point1.high, 110.0);
        assert_eq!(point1.low, 100.0);
        assert_eq!(point1.close, 110.0);
        assert_eq!(point1.count, 2);
    }

    #[test]
    fn test_resolution_level_window() {
        let mut level = ResolutionLevel::new(1_000_000_000, 0.0, 10.0);

        for i in 0..10 {
            level.add_point(AggregatedPoint::from_single(
                i * 1_000_000_000,
                100.0 + i as f32,
            ));
        }

        let points = level.get_window_points(2_000_000_000, 7_000_000_000);
        assert_eq!(points.len(), 6); // Points at 2, 3, 4, 5, 6, 7 seconds
    }

    #[test]
    fn test_lod_selection() {
        let mut series = MultiResolutionSeries::new("TEST".to_string());

        // Add multiple resolution levels
        let mut second_level = ResolutionLevel::new(1_000_000_000, 0.0, 1.0);
        let mut minute_level = ResolutionLevel::new(60_000_000_000, 1.0, 60.0);

        series.add_level(second_level);
        series.add_level(minute_level);

        // Test selection for 1 hour window at 1000px width
        let selected = series.select_lod(0, 3600_000_000_000, 1000.0);
        assert!(selected.is_some());

        // Should select minute level for hour-long window
        assert_eq!(selected.unwrap().interval_ns, 60_000_000_000);
    }

    #[test]
    fn test_create_level_from_raw() {
        let mut series = MultiResolutionSeries::new("TEST".to_string());

        // Create raw data points (1 point per second for 100 seconds)
        let raw_points: Vec<(u64, f32)> = (0..100)
            .map(|i| (i * 1_000_000_000, 100.0 + (i as f32).sin() * 10.0))
            .collect();

        // Create a 10-second resolution level
        series.create_level_from_raw(&raw_points, 10_000_000_000, 0.0, 10.0);

        assert_eq!(series.levels.len(), 1);
        assert_eq!(series.levels[0].points.len(), 10); // 100 seconds / 10 = 10 points
    }

    #[test]
    fn test_progressive_loader() {
        let mut loader = ProgressiveLodLoader::new("TEST".to_string());

        // Set raw data
        let raw_data: Vec<(u64, f32)> = (0..1000)
            .map(|i| (i * 1_000_000_000, 100.0 + (i as f32 * 0.1).sin() * 20.0))
            .collect();

        loader.set_raw_data(raw_data);

        // Process first level
        assert!(loader.process_next_level());
        assert!(loader.get_progress() > 0.0);

        // Process remaining levels
        while !loader.is_complete() {
            loader.process_next_level();
        }

        assert_eq!(loader.get_progress(), 100.0);
        assert!(!loader.get_series().levels.is_empty());
    }
}
