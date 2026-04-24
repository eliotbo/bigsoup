/// The goal of the whole code is to build a calendar map that compresses real-world market time (in UTC) into a continuous axis that skips non-trading periods like weekends and nights. Internally, all timestamps stay in UTC for consistency, but the decision of what counts as "in-hours" is made in NYC local time, respecting weekends and daylight savings changes. The output is a set of contiguous UTC spans, each tagged with its "compressed" axis offset. With this structure, you can quickly map back and forth between real UTC timestamps and their compressed axis positions, so that market plots can display trading activity without gaps where the market is closed.

///   **Examples:**
///   Weekend filter:
///   11 Sep | 12 Sep | 13 Sep | 14 Sep | 15 Sep →
///   11 Sep | 12 Sep | 15 Sep | 16 Sep | 17 Sep (13 and 14 were weekend days)
///
///   Night filter (8pm-8am):
///   14:00 | 16:00 | 18:00 | 20:00 | 22:00 | 00:00 | 2:00
///   →
///   14:00 | 16:00 | 18:00 | 20:00 | 8:00 | 10:00 | 12:00
use chrono::{Datelike, NaiveDate, TimeZone};
use chrono_tz::{America::New_York, Tz};
use vizza::{LodLevel, TimeTickSpacing, select_time_spacing};

/// Closed-open UTC span with cumulative compressed offset.
#[derive(Debug, Clone)]
pub struct Span {
    pub real_start: i64, // UTC seconds [inclusive)
    pub real_end: i64,   // UTC seconds (exclusive]
    pub axis_start: i64, // compressed seconds before this span
}

#[derive(Debug, Clone, Default)]
pub struct CalendarMap {
    pub spans: Vec<Span>, // sorted, non-overlapping
}

impl CalendarMap {
    #[inline]
    pub fn real_to_axis(&self, t_utc: i64) -> Option<i64> {
        let idx = self
            .spans
            .binary_search_by(|s| {
                if t_utc < s.real_start {
                    std::cmp::Ordering::Greater
                } else if t_utc >= s.real_end {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()?;
        let s = &self.spans[idx];
        Some(s.axis_start + (t_utc - s.real_start))
    }

    #[inline]
    pub fn axis_to_real(&self, u: i64) -> Option<i64> {
        let idx = self
            .spans
            .binary_search_by(|s| {
                let len = s.real_end - s.real_start;
                if u < s.axis_start {
                    std::cmp::Ordering::Greater
                } else if u >= s.axis_start + len {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()?;
        let s = &self.spans[idx];
        Some(s.real_start + (u - s.axis_start))
    }

    #[inline]
    pub fn axis_len(&self) -> i64 {
        self.spans
            .last()
            .map(|s| s.axis_start + (s.real_end - s.real_start))
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
pub struct Rules {
    /// Use NYC for inclusion (display) logic; store everything in UTC.
    pub tz: Tz, // usually America/New_York
    pub drop_weekends: bool, // true to exclude Sat/Sun entirely
    pub day_start_h: u8,     // e.g., 8  (08:00 local)
    pub day_end_h: u8,       // e.g., 20 (20:00 local, exclusive)
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            tz: New_York,
            drop_weekends: true,
            day_start_h: 8,
            day_end_h: 20,
        }
    }
}

#[inline]
fn is_weekend_local(date: NaiveDate, tz: Tz) -> bool {
    use chrono::Weekday::{Sat, Sun};
    // Get weekday from local midnight (earliest mapping)
    let local_midnight = tz
        .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .earliest()
        .expect("local midnight should exist");
    matches!(local_midnight.weekday(), Sat | Sun)
}

/// Map a single NYC local window [date@start_h, date@end_h) to **one UTC span**, if it exists.
/// - **Ignores** fall-back double windows (we take the earliest mapping).
/// - If the window is nonexistent (spring-forward), returns None.
#[inline]
fn local_window_utc_simple(date: NaiveDate, rules: &Rules) -> Option<(i64, i64)> {
    let tz = rules.tz;
    let s = tz
        .with_ymd_and_hms(
            date.year(),
            date.month(),
            date.day(),
            rules.day_start_h as u32,
            0,
            0,
        )
        .earliest()?;
    let e = tz
        .with_ymd_and_hms(
            date.year(),
            date.month(),
            date.day(),
            rules.day_end_h as u32,
            0,
            0,
        )
        .earliest()?;
    let su = s.timestamp();
    let eu = e.timestamp();
    (eu > su).then_some((su, eu))
}

/// Build the compressed time map over [start_utc, end_utc) using a single NYC window per weekday.
/// Complexity: O(D), where D = number of local days in range. Memory: O(S), S ≈ D.
pub fn build_calendar_map_range(start_utc: i64, end_utc: i64, rules: &Rules) -> CalendarMap {
    assert!(end_utc >= start_utc);
    if start_utc == end_utc {
        return CalendarMap::default();
    }

    let tz = rules.tz;
    // Anchor to local dates covering the UTC range.
    let start_local = tz.timestamp_opt(start_utc, 0).earliest().unwrap();
    let end_local = tz
        .timestamp_opt(end_utc.max(start_utc + 1), 0)
        .latest()
        .unwrap();

    let mut date = start_local.date_naive();
    let end_date = end_local.date_naive();

    let mut spans: Vec<Span> = Vec::new();
    let mut axis_acc: i64 = 0;

    while date <= end_date {
        if !(rules.drop_weekends && is_weekend_local(date, tz)) {
            if let Some((su, eu)) = local_window_utc_simple(date, rules) {
                // Clamp to requested UTC range and record.
                let rs = su.max(start_utc);
                let re = eu.min(end_utc);
                if re > rs {
                    let len = re - rs;
                    spans.push(Span {
                        real_start: rs,
                        real_end: re,
                        axis_start: axis_acc,
                    });
                    axis_acc += len;
                }
            }
        }
        date = date.succ_opt().unwrap();
    }

    CalendarMap { spans }
}

/// Compress a sequence of real timestamps through the calendar map.
/// Returns a vector of compressed axis positions (Some) or None for filtered-out times.
pub fn compress_sequence(timestamps: &[i64], map: &CalendarMap) -> Vec<Option<i64>> {
    timestamps.iter().map(|&t| map.real_to_axis(t)).collect()
}

/// Decompress a sequence of axis positions back to real timestamps.
/// Returns a vector of real UTC timestamps (Some) or None for invalid axis positions.
pub fn decompress_sequence(axis_positions: &[i64], map: &CalendarMap) -> Vec<Option<i64>> {
    axis_positions
        .iter()
        .map(|&u| map.axis_to_real(u))
        .collect()
}

/// Choose appropriate LOD level based on pixel density and viewport.
///
/// This function calculates the optimal tick spacing to achieve roughly TARGET_PX
/// pixels between ticks, while respecting the minimum LOD constraint.
pub fn choose_lod(px_per_axis_sec: f64, current_lod_secs: f64) -> LodLevel {
    const TARGET_PX: f64 = 100.0;

    let desired_sec = (TARGET_PX / px_per_axis_sec).max(current_lod_secs);

    // Ladder of available LOD levels in ascending order
    let ladder = [
        LodLevel::S1,  // 1 second
        LodLevel::S15, // 15 seconds
        LodLevel::S30, // 30 seconds
        LodLevel::M1,  // 1 minute
        LodLevel::M5,  // 5 minutes
        LodLevel::M15, // 15 minutes
        LodLevel::M30, // 30 minutes
        LodLevel::H1,  // 1 hour
        LodLevel::H4,  // 4 hours
        LodLevel::D1,  // 1 day
        LodLevel::W1,  // 1 week
    ];

    // Snap UP to the first >= desired_sec and also >= current_lod_secs
    let need = desired_sec.max(current_lod_secs);
    ladder
        .iter()
        .copied()
        .find(|lod| lod.seconds() >= need)
        .unwrap_or(LodLevel::M1)
}

/// Generate plot ticks in the compressed (axis) space.
///
/// This function:
/// 1. Calculates visible seconds from zoom level (zoom x1 = 100s)
/// 2. Determines expected number of ticks based on visible seconds and LOD
/// 3. Generates regular ticks in real time using the given LOD interval
/// 4. Maps each tick through the calendar compression
/// 5. Returns only the ticks that fall within trading hours (non-filtered times)
///
/// The result is a set of axis positions that can be used to draw grid lines
/// on a compressed time chart.
#[derive(Clone, Debug)]
pub struct GridTicks {
    pub axis_positions: Vec<i64>,
    pub spacing: TimeTickSpacing,
    pub axis_start: f64,
    pub visible_span_seconds: f64,
}

pub fn generate_compressed_plot_ticks(
    viewport_width_px: f64,
    pixels_per_second: f64,
    map: &CalendarMap,
) -> Option<GridTicks> {
    if map.spans.is_empty() || viewport_width_px <= 0.0 {
        return None;
    }

    if !pixels_per_second.is_finite() || pixels_per_second <= 0.0 {
        return None;
    }

    let axis_len = map.axis_len() as f64;
    if axis_len <= 0.0 {
        return None;
    }

    let visible_span_seconds = (viewport_width_px / pixels_per_second).min(axis_len);
    let axis_end = axis_len;
    let axis_start = (axis_end - visible_span_seconds).max(0.0);

    let span_seconds = visible_span_seconds.max(1.0);
    let spacing = select_time_spacing(span_seconds, None);

    let real_window_start = axis_to_real_or_default(map, axis_start, false);
    let real_window_end = axis_to_real_or_default(map, axis_end, true);

    let ticks_real = spacing.generate_ticks(real_window_start as f64, real_window_end as f64);

    let mut ticks_axis = Vec::new();
    for tick_real in ticks_real {
        let real_sec = tick_real.round() as i64;
        if let Some(axis_pos) = project_tick_to_axis(map, real_sec) {
            let axis_pos_f64 = axis_pos as f64;
            if axis_pos_f64 >= axis_start && axis_pos_f64 <= axis_end {
                ticks_axis.push(axis_pos);
            }
        }
    }

    ticks_axis.sort_unstable();
    ticks_axis.dedup();

    Some(GridTicks {
        axis_positions: ticks_axis,
        spacing,
        axis_start,
        visible_span_seconds,
    })
}

fn axis_to_real_or_default(map: &CalendarMap, axis_pos: f64, prefer_end: bool) -> i64 {
    let floor_axis = axis_pos.floor() as i64;
    if let Some(ts) = map.axis_to_real(floor_axis) {
        return ts;
    }

    let ceil_axis = axis_pos.ceil() as i64;
    if let Some(ts) = map.axis_to_real(ceil_axis) {
        return ts;
    }

    if prefer_end {
        map.spans.last().map(|span| span.real_end).unwrap_or(0)
    } else {
        map.spans.first().map(|span| span.real_start).unwrap_or(0)
    }
}

fn project_tick_to_axis(map: &CalendarMap, real_tick: i64) -> Option<i64> {
    if let Some(axis_pos) = map.real_to_axis(real_tick) {
        return Some(axis_pos);
    }

    map.spans
        .iter()
        .find(|span| real_tick < span.real_start)
        .map(|span| span.axis_start)
}

fn main() {
    println!("Run tests with: cargo test --test compress_test");
}
