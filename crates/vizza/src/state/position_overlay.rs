/// Defines a time span that should be highlighted on top of the viewport.
///
/// Timestamps are expressed in nanoseconds since the Unix epoch to match the
/// `lod::PlotCandle` representation used throughout the renderer.
#[derive(Debug, Clone)]
pub struct PositionOverlay {
    /// Inclusive start timestamp in nanoseconds.
    pub start_ts: i64,
    /// Inclusive end timestamp in nanoseconds.
    pub end_ts: i64,
    /// RGBA color to fill the overlay with.
    pub color: [f32; 4],
}

impl PositionOverlay {
    /// Create a new overlay span with the provided color.
    pub fn new(start_ts: i64, end_ts: i64, color: [f32; 4]) -> Self {
        let (start_ts, end_ts) = if end_ts >= start_ts {
            (start_ts, end_ts)
        } else {
            (end_ts, start_ts)
        };

        Self {
            start_ts,
            end_ts,
            color,
        }
    }

    /// Convenience constructor for a green "long" overlay.
    pub fn long(start_ts: i64, end_ts: i64) -> Self {
        Self::new(start_ts, end_ts, [0.15, 0.65, 0.30, 0.28])
    }

    /// Convenience constructor for a red "short" overlay.
    pub fn short(start_ts: i64, end_ts: i64) -> Self {
        Self::new(start_ts, end_ts, [0.75, 0.25, 0.25, 0.28])
    }

    /// Return `true` if the overlay spans a non-zero time interval.
    pub fn is_valid(&self) -> bool {
        self.end_ts > self.start_ts
    }

    /// Adjust the overlay opacity without changing its hue.
    pub fn with_opacity(mut self, alpha: f32) -> Self {
        self.color[3] = alpha.clamp(0.0, 1.0);
        self
    }
}
