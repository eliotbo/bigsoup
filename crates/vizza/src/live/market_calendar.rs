//! Market calendar logic for determining trading hours.

use chrono::{DateTime, Datelike, NaiveTime, Utc, Weekday};

/// Handles market hours and trading calendar logic.
#[derive(Debug, Clone)]
pub struct MarketCalendar {
    /// Market open time (ET)
    market_open: NaiveTime,
    /// Market close time (ET)
    market_close: NaiveTime,
}

impl MarketCalendar {
    /// Create a new market calendar with US equity market hours (9:30-16:00 ET).
    pub fn us_equity() -> Self {
        Self {
            market_open: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
            market_close: NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
        }
    }

    /// Check if the given timestamp is during market hours.
    ///
    /// # Arguments
    /// * `timestamp_ns` - Time in nanoseconds since epoch
    ///
    /// # Returns
    /// `true` if market is open, `false` otherwise
    pub fn is_market_open(&self, timestamp_ns: i64) -> bool {
        let dt =
            DateTime::from_timestamp(timestamp_ns / 1_000_000_000, 0).unwrap_or_else(|| Utc::now());

        // Check if weekend
        if self.is_weekend(&dt) {
            return false;
        }

        // For simplicity, we're using UTC time as a proxy
        // In production, this should properly handle ET timezone
        let time = dt.time();

        // Simple check: assume UTC is close enough for now
        // Real implementation would convert to ET
        time >= self.market_open && time < self.market_close
    }

    /// Check if the given datetime is a weekend.
    fn is_weekend(&self, dt: &DateTime<Utc>) -> bool {
        matches!(dt.weekday(), Weekday::Sat | Weekday::Sun)
    }

    /// Check if there's likely a holiday/gap based on time since last bar.
    ///
    /// # Arguments
    /// * `last_bar_ns` - Timestamp of last historical bar in nanoseconds
    /// * `current_ns` - Current timestamp in nanoseconds
    ///
    /// # Returns
    /// `true` if gap suggests a major holiday or market closure (> 7 days)
    ///
    /// Note: A 5-day gap over a weekend is normal (Fri close to Mon open + weekend).
    /// Only consider it a holiday if gap is significant (> 1 week).
    pub fn is_likely_holiday(&self, last_bar_ns: i64, current_ns: i64) -> bool {
        let gap_ns = current_ns - last_bar_ns;
        let gap_days = gap_ns / (24 * 3600 * 1_000_000_000);

        // Only flag as holiday if gap is very large (> 7 days)
        // Normal weekend gaps (Fri 4pm to Mon 9:30am) are ~2.5 days
        // Even with a long weekend, 7+ days indicates a real closure
        gap_days > 7
    }
}

impl Default for MarketCalendar {
    fn default() -> Self {
        Self::us_equity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weekend_detection() {
        let cal = MarketCalendar::us_equity();

        // Saturday
        let saturday = DateTime::from_timestamp(1704585600, 0).unwrap(); // 2024-01-07 00:00:00 UTC (Sat)
        assert!(!cal.is_market_open(saturday.timestamp_nanos_opt().unwrap()));

        // Sunday
        let sunday = DateTime::from_timestamp(1704672000, 0).unwrap(); // 2024-01-08 00:00:00 UTC (Sun)
        assert!(!cal.is_market_open(sunday.timestamp_nanos_opt().unwrap()));
    }
}
