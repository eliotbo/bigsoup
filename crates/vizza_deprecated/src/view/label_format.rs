use crate::time_spacing::{TimeTickSpacing, TimeUnit};
use chrono::{DateTime, Datelike, Utc};
use chrono_tz::America::New_York;

/// Format a timestamp for display based on spacing unit
pub fn format_time_label(
    timestamp_secs: i64,
    spacing: &TimeTickSpacing,
    visible_span_seconds: f64,
) -> String {
    let dt =
        DateTime::<Utc>::from_timestamp(timestamp_secs, 0).map(|dt| dt.with_timezone(&New_York));

    let Some(nyc_dt) = dt else {
        return format!("T{}", timestamp_secs);
    };

    match spacing.unit {
        TimeUnit::Year => format!("{}", nyc_dt.year()),
        TimeUnit::Month => {
            if visible_span_seconds > 30.0 * 24.0 * 3600.0 {
                nyc_dt.format("%b '%y").to_string()
            } else {
                nyc_dt.format("%b").to_string()
            }
        }
        TimeUnit::Day => nyc_dt.format("%b %d").to_string(),
        TimeUnit::Hour => {
            if visible_span_seconds > 2.0 * 24.0 * 3600.0 {
                nyc_dt.format("%a %H:%M").to_string()
            } else {
                nyc_dt.format("%H:%M").to_string()
            }
        }
        TimeUnit::Minute => nyc_dt.format("%H:%M").to_string(),
        TimeUnit::Second => nyc_dt.format("%H:%M:%S").to_string(),
    }
}
