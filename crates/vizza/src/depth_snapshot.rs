/// A snapshot of the limit order book depth, suitable for rendering.
/// Each entry is (price, total_quantity) at that price level.
#[derive(Clone, Debug, Default)]
pub struct DepthSnapshot {
    /// Bid levels sorted descending by price (best bid first).
    pub bids: Vec<(f32, f32)>,
    /// Ask levels sorted ascending by price (best ask first).
    pub asks: Vec<(f32, f32)>,
}
