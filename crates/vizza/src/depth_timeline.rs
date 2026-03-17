/// Data model for the depth timeline graph.

/// A single snapshot of the order book at one point in time.
#[derive(Clone, Debug)]
pub struct DepthTimelineEntry {
    /// Simulation tick or Unix timestamp for the X axis.
    pub tick: u64,
    /// Bid levels: Vec<(price, quantity)> sorted descending by price (best bid first).
    pub bids: Vec<(f32, f32)>,
    /// Ask levels: Vec<(price, quantity)> sorted ascending by price (best ask first).
    pub asks: Vec<(f32, f32)>,
}

/// A series of order book snapshots over time.
#[derive(Clone, Debug)]
pub struct DepthTimeline {
    /// Snapshots in chronological order, taken every N simulation ticks.
    pub snapshots: Vec<DepthTimelineEntry>,
}

/// State for the depth timeline viewport.
pub struct DepthTimelineState {
    /// The full timeline data.
    pub timeline: DepthTimeline,
    /// Index of the rightmost visible column (exclusive).
    pub visible_right: usize,
    /// Number of columns visible in the viewport.
    pub visible_count: usize,
    /// Visible price range.
    pub price_min: f32,
    pub price_max: f32,
    /// Whether auto Y-scaling is enabled.
    pub auto_y_scale: bool,
    /// Pixels per column.
    pub column_width_px: f32,
}

impl DepthTimelineState {
    pub fn new(timeline: DepthTimeline, visible_count: usize, column_width_px: f32) -> Self {
        let visible_right = timeline.snapshots.len();
        let mut state = Self {
            timeline,
            visible_right,
            visible_count,
            price_min: 0.0,
            price_max: 0.0,
            auto_y_scale: true,
            column_width_px,
        };
        state.auto_scale_y();
        state
    }

    /// Return the visible slice of snapshots.
    pub fn visible_snapshots(&self) -> &[DepthTimelineEntry] {
        let start = self.visible_right.saturating_sub(self.visible_count);
        let end = self.visible_right.min(self.timeline.snapshots.len());
        if start >= end {
            return &[];
        }
        &self.timeline.snapshots[start..end]
    }

    /// Index of the first visible snapshot.
    pub fn visible_left(&self) -> usize {
        self.visible_right.saturating_sub(self.visible_count)
    }

    /// Pan by `delta_cols` columns (positive = pan right / show later data).
    pub fn pan_x(&mut self, delta_cols: i32) {
        let new_right = self.visible_right as i64 + delta_cols as i64;
        let max_right = self.timeline.snapshots.len();
        let min_right = self.visible_count.min(max_right);
        self.visible_right = (new_right as usize).clamp(min_right, max_right);
        if self.auto_y_scale {
            self.auto_scale_y();
        }
    }

    /// Recompute price_min/price_max from visible data with 5% margin.
    pub fn auto_scale_y(&mut self) {
        let snapshots = self.visible_snapshots();
        if snapshots.is_empty() {
            return;
        }

        let mut lo = f32::MAX;
        let mut hi = f32::MIN;

        for snap in snapshots {
            for &(price, _) in &snap.bids {
                lo = lo.min(price);
                hi = hi.max(price);
            }
            for &(price, _) in &snap.asks {
                lo = lo.min(price);
                hi = hi.max(price);
            }
        }

        if lo == f32::MAX || hi == f32::MIN {
            return;
        }

        let range = (hi - lo).max(0.01);
        let margin = range * 0.05;
        self.price_min = lo - margin;
        self.price_max = hi + margin;
    }
}
