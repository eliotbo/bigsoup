/// Defines a horizontal price-level quad for stop-loss or take-profit visualization.
///
/// Unlike `PositionOverlay` which spans the full height of the viewport for a time range,
/// `PriceLevelQuad` renders a horizontal rectangle between two price levels (Y values)
/// over a specified time range.
///
/// Timestamps are expressed in nanoseconds since the Unix epoch to match the
/// `lod::PlotCandle` representation used throughout the renderer.
#[derive(Debug, Clone)]
pub struct PriceLevelQuad {
    /// Inclusive start timestamp in nanoseconds.
    pub start_ts: i64,
    /// Inclusive end timestamp in nanoseconds.
    pub end_ts: i64,
    /// Starting price level (e.g., buy price).
    pub y_start: f32,
    /// Ending price level (e.g., stop-loss or take-profit price).
    pub y_end: f32,
    /// RGBA color to fill the quad with.
    pub color: [f32; 4],
}

impl PriceLevelQuad {
    /// Create a new price level quad with the provided parameters.
    pub fn new(start_ts: i64, end_ts: i64, y_start: f32, y_end: f32, color: [f32; 4]) -> Self {
        let (start_ts, end_ts) = if end_ts >= start_ts {
            (start_ts, end_ts)
        } else {
            (end_ts, start_ts)
        };

        let (y_start, y_end) = if y_end >= y_start {
            (y_start, y_end)
        } else {
            (y_end, y_start)
        };

        Self {
            start_ts,
            end_ts,
            y_start,
            y_end,
            color,
        }
    }

    /// Convenience constructor for a red stop-loss quad.
    ///
    /// Renders a semi-transparent red quad between the buy price and stop-loss price.
    pub fn stop_loss(start_ts: i64, end_ts: i64, buy_price: f32, stop_price: f32) -> Self {
        Self::new(
            start_ts,
            end_ts,
            buy_price,
            stop_price,
            [0.75, 0.25, 0.25, 0.30],
        )
    }

    /// Convenience constructor for a green take-profit quad.
    ///
    /// Renders a semi-transparent green quad between the buy price and take-profit price.
    pub fn take_profit(start_ts: i64, end_ts: i64, buy_price: f32, target_price: f32) -> Self {
        Self::new(
            start_ts,
            end_ts,
            buy_price,
            target_price,
            [0.25, 0.75, 0.30, 0.30],
        )
    }

    /// Return `true` if the quad spans a non-zero time interval and price range.
    pub fn is_valid(&self) -> bool {
        self.end_ts > self.start_ts && (self.y_end - self.y_start).abs() > f32::EPSILON
    }

    /// Adjust the quad opacity without changing its hue.
    pub fn with_opacity(mut self, alpha: f32) -> Self {
        self.color[3] = alpha.clamp(0.0, 1.0);
        self
    }
}
