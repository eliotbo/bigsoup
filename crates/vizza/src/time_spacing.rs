use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, NaiveDateTime, Timelike};

const SECONDS_PER_MINUTE: f64 = 60.0;
const SECONDS_PER_HOUR: f64 = 3_600.0;
const SECONDS_PER_DAY: f64 = 86_400.0;
const SECONDS_PER_MONTH_APPROX: f64 = SECONDS_PER_DAY * 30.0;
const SECONDS_PER_YEAR_APPROX: f64 = SECONDS_PER_DAY * 365.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Month,
    Year,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeTickSpacing {
    pub unit: TimeUnit,
    pub step: u32,
}

impl TimeTickSpacing {
    pub fn approx_seconds(&self) -> f64 {
        let base = match self.unit {
            TimeUnit::Second => 1.0,
            TimeUnit::Minute => SECONDS_PER_MINUTE,
            TimeUnit::Hour => SECONDS_PER_HOUR,
            TimeUnit::Day => SECONDS_PER_DAY,
            TimeUnit::Month => SECONDS_PER_MONTH_APPROX,
            TimeUnit::Year => SECONDS_PER_YEAR_APPROX,
        };
        base * self.step as f64
    }

    pub fn generate_ticks(&self, start: f64, end: f64) -> Vec<f64> {
        if !start.is_finite() || !end.is_finite() || end <= start {
            return Vec::new();
        }

        let start_dt = seconds_to_datetime(start);
        let end_dt = seconds_to_datetime(end);

        let mut ticks = Vec::new();
        let mut current = self.align_down(start_dt);

        while current > start_dt {
            if let Some(prev) = self.step_by(current, -1) {
                current = prev;
            } else {
                break;
            }
        }

        while current < start_dt {
            if let Some(next) = self.step_by(current, 1) {
                current = next;
            } else {
                break;
            }
        }

        let mut guard = 0;
        const MAX: usize = 256;
        while current <= end_dt && guard < MAX {
            ticks.push(datetime_to_seconds(current));
            if let Some(next) = self.step_by(current, 1) {
                current = next;
            } else {
                break;
            }
            guard += 1;
        }

        ticks
    }

    fn align_down(&self, dt: NaiveDateTime) -> NaiveDateTime {
        match self.unit {
            TimeUnit::Second => {
                let second = dt.second();
                let aligned = second - (second % self.step);
                dt.date()
                    .and_hms_opt(dt.hour(), dt.minute(), aligned)
                    .unwrap()
            }
            TimeUnit::Minute => {
                let minute = dt.minute();
                let aligned = minute - (minute % self.step);
                dt.date().and_hms_opt(dt.hour(), aligned, 0).unwrap()
            }
            TimeUnit::Hour => {
                let hour = dt.hour();
                let aligned = hour - (hour % self.step);
                dt.date().and_hms_opt(aligned, 0, 0).unwrap()
            }
            TimeUnit::Day => {
                let day0 = dt.day0();
                let aligned = day0 - (day0 % self.step);
                dt.date()
                    .with_day0(aligned)
                    .unwrap_or_else(|| NaiveDate::from_ymd_opt(dt.year(), dt.month(), 1).unwrap())
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
            }
            TimeUnit::Month => {
                let month0 = dt.month0();
                let aligned = month0 - (month0 % self.step);
                NaiveDate::from_ymd_opt(dt.year(), aligned + 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
            }
            TimeUnit::Year => {
                let step = self.step.max(1) as i32;
                let year = dt.year();
                let aligned = year.div_euclid(step) * step;
                NaiveDate::from_ymd_opt(aligned, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
            }
        }
    }

    fn step_by(&self, dt: NaiveDateTime, steps: i32) -> Option<NaiveDateTime> {
        if steps == 0 {
            return Some(dt);
        }

        match self.unit {
            TimeUnit::Second => {
                dt.checked_add_signed(Duration::seconds((self.step as i64) * steps as i64))
            }
            TimeUnit::Minute => {
                dt.checked_add_signed(Duration::minutes((self.step as i64) * steps as i64))
            }
            TimeUnit::Hour => {
                dt.checked_add_signed(Duration::hours((self.step as i64) * steps as i64))
            }
            TimeUnit::Day => {
                dt.checked_add_signed(Duration::days((self.step as i64) * steps as i64))
            }
            TimeUnit::Month => step_months(dt, (self.step as i64) * steps as i64),
            TimeUnit::Year => step_months(dt, (self.step as i64 * 12) * steps as i64),
        }
    }
}

pub fn choose_time_unit(span_seconds: f64) -> TimeUnit {
    if span_seconds > 2.0 * SECONDS_PER_YEAR_APPROX {
        TimeUnit::Year
    } else if span_seconds >= 2.0 * SECONDS_PER_MONTH_APPROX {
        TimeUnit::Month
    } else if span_seconds >= 2.0 * SECONDS_PER_DAY {
        TimeUnit::Day
    } else if span_seconds >= 2.0 * SECONDS_PER_HOUR {
        TimeUnit::Hour
    } else if span_seconds >= 2.0 * SECONDS_PER_MINUTE {
        TimeUnit::Minute
    } else {
        TimeUnit::Second
    }
}

/// Select appropriate time spacing for the given span, with hysteresis
pub fn select_time_spacing(span_seconds: f64, prev: Option<TimeTickSpacing>) -> TimeTickSpacing {
    const MIN_TICKS: usize = 4;
    const MAX_TICKS: usize = 10;

    let unit = choose_time_unit(span_seconds);

    // Hysteresis: if previous spacing still valid, keep it
    if let Some(prev_spacing) = prev {
        if prev_spacing.unit == unit {
            let approx = span_seconds / prev_spacing.approx_seconds();
            if approx >= (MIN_TICKS as f64 - 1.0) && approx <= (MAX_TICKS as f64 + 1.0) {
                return prev_spacing;
            }
        }
    }

    // Find best spacing for target tick count
    let desired = ((MIN_TICKS + MAX_TICKS) / 2) as f64;
    let mut best = TimeTickSpacing {
        unit,
        step: candidate_steps(unit)[0],
    };
    let mut best_score = f64::MAX;

    for &step in candidate_steps(unit) {
        let spacing = TimeTickSpacing { unit, step };
        let count = span_seconds / spacing.approx_seconds();
        let in_bounds = count >= MIN_TICKS as f64 && count <= MAX_TICKS as f64;
        let score = (count - desired).abs() + if in_bounds { 0.0 } else { 1_000.0 };
        if score < best_score {
            best = spacing;
            best_score = score;
        }
    }

    best
}

pub fn candidate_steps(unit: TimeUnit) -> &'static [u32] {
    match unit {
        TimeUnit::Second => &[1, 2, 5, 10, 15, 30],
        TimeUnit::Minute => &[1, 2, 5, 10, 15, 30],
        TimeUnit::Hour => &[1, 2, 3, 4, 6, 12],
        TimeUnit::Day => &[1, 2, 3, 5, 7, 10, 14],
        TimeUnit::Month => &[1, 2, 3, 6, 12],
        TimeUnit::Year => &[1, 2, 5, 10, 20],
    }
}

fn step_months(dt: NaiveDateTime, total_months: i64) -> Option<NaiveDateTime> {
    if total_months == 0 {
        return Some(dt);
    }

    let months = Months::new(total_months.unsigned_abs() as u32);
    if total_months.is_positive() {
        dt.checked_add_months(months)
    } else {
        dt.checked_sub_months(months)
    }
}

fn seconds_to_datetime(secs: f64) -> NaiveDateTime {
    let whole = secs.floor() as i64;
    let nanos = ((secs - secs.floor()) * 1_000_000_000.0) as i64;
    let nanos = nanos.clamp(0, 999_999_999) as u32;
    DateTime::from_timestamp(whole, nanos).unwrap().naive_utc()
}

fn datetime_to_seconds(dt: NaiveDateTime) -> f64 {
    dt.and_utc().timestamp() as f64
        + f64::from(dt.and_utc().timestamp_subsec_nanos()) / 1_000_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_reasonable_tick_count() {
        let spacing = TimeTickSpacing {
            unit: TimeUnit::Hour,
            step: 6,
        };
        let start = 1_600_000_000.0;
        let end = start + 7.0 * SECONDS_PER_DAY;
        let ticks = spacing.generate_ticks(start, end);
        assert!(ticks.len() > 0);
        assert!(ticks.len() < 64);
    }
}
