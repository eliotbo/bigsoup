/// A line overlay rendered behind candlestick bars.
///
/// Each overlay is a series of `(timestamp_ns, price)` points connected by line
/// segments.  Typical use case: plotting moving averages (EMA, SMA) or other
/// continuous indicators on top of the price chart.
///
/// Timestamps are expressed in **nanoseconds** since the Unix epoch to match the
/// `lod::PlotCandle` representation used throughout the renderer.
#[derive(Debug, Clone)]
pub struct LineOverlay {
    /// Ordered series of `(timestamp_ns, price)` pairs.
    /// Must be sorted by timestamp for correct rendering.
    pub points: Vec<(i64, f32)>,
    /// RGBA color for the line.
    pub color: [f32; 4],
}

impl LineOverlay {
    /// Create a new line overlay.
    pub fn new(points: Vec<(i64, f32)>, color: [f32; 4]) -> Self {
        Self { points, color }
    }
}
