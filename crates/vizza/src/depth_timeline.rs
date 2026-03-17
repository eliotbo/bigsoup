/// Data model for the depth timeline graph.

use crate::config::ColorPalette;

// ── Shared GPU types ──────────────────────────────────────────────────

/// Per-instance GPU data for the depth timeline shader.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DepthTimelineInstance {
    pub column_index: f32,
    pub price: f32,
    pub log_quantity: f32,
    pub color_r: f32,
    pub color_g: f32,
    pub color_b: f32,
}

/// Uniform block for the depth timeline shader.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DepthTimelineUniform {
    pub price_min: f32,
    pub price_max: f32,
    pub col_start: f32,
    pub col_count: f32,
    pub max_log_qty: f32,
    pub window_w: f32,
    pub window_h: f32,
    pub column_width_px: f32,
    pub margin_left: f32,
    pub margin_bottom: f32,
    pub margin_top: f32,
    pub margin_right: f32,
}

/// Chart margin configuration.
#[derive(Copy, Clone, Debug)]
pub(crate) struct ChartMargins {
    pub left: f32,
    pub bottom: f32,
    pub top: f32,
    pub right: f32,
}

pub(crate) const MAX_INSTANCES: usize = 100_000;

/// Default margins used by the headless PNG renderer.
pub(crate) const HEADLESS_MARGINS: ChartMargins = ChartMargins {
    left: 70.0,
    bottom: 24.0,
    top: 0.0,
    right: 0.0,
};

/// Build GPU instances from the visible portion of the state.
pub(crate) fn prepare_instances(
    state: &DepthTimelineState,
    palette: &ColorPalette,
    window_w: f32,
    window_h: f32,
    margins: ChartMargins,
) -> (Vec<DepthTimelineInstance>, DepthTimelineUniform) {
    let (start, end) = state.visible_range();
    let snapshots = &state.timeline.snapshots[start..end];
    let bid_color = palette.candle_up_market;
    let ask_color = palette.candle_down_market;
    let col_start = state.scroll_left();
    let col_count = state.visible_count_f32();

    let mut instances = Vec::new();
    let mut max_log_qty: f32 = 0.0;

    for (i, snap) in snapshots.iter().enumerate() {
        let col_idx = start as f32 + i as f32;
        for &(price, qty) in &snap.bids {
            if price < state.price_min || price > state.price_max {
                continue;
            }
            let log_qty = (1.0 + qty).log10();
            max_log_qty = max_log_qty.max(log_qty);
            instances.push(DepthTimelineInstance {
                column_index: col_idx,
                price,
                log_quantity: log_qty,
                color_r: bid_color[0],
                color_g: bid_color[1],
                color_b: bid_color[2],
            });
        }
        for &(price, qty) in &snap.asks {
            if price < state.price_min || price > state.price_max {
                continue;
            }
            let log_qty = (1.0 + qty).log10();
            max_log_qty = max_log_qty.max(log_qty);
            instances.push(DepthTimelineInstance {
                column_index: col_idx,
                price,
                log_quantity: log_qty,
                color_r: ask_color[0],
                color_g: ask_color[1],
                color_b: ask_color[2],
            });
        }
    }

    if max_log_qty <= 0.0 {
        max_log_qty = 1.0;
    }

    instances.truncate(MAX_INSTANCES);

    let uniform = DepthTimelineUniform {
        price_min: state.price_min,
        price_max: state.price_max,
        col_start,
        col_count,
        max_log_qty,
        window_w,
        window_h,
        column_width_px: state.column_width_px,
        margin_left: margins.left,
        margin_bottom: margins.bottom,
        margin_top: margins.top,
        margin_right: margins.right,
    };

    (instances, uniform)
}

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
    /// Continuous scroll position: the fractional column index of the right edge of the viewport.
    pub scroll_right: f32,
    /// Number of columns visible in the viewport (integer count for data slicing).
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
        let scroll_right = timeline.snapshots.len() as f32;
        let mut state = Self {
            timeline,
            scroll_right,
            visible_count,
            price_min: 0.0,
            price_max: 0.0,
            auto_y_scale: true,
            column_width_px,
        };
        state.auto_scale_y();
        state
    }

    /// Fractional left edge of the viewport in column space.
    pub fn scroll_left(&self) -> f32 {
        self.scroll_right - self.visible_count_f32()
    }

    /// Visible count as f32.
    pub fn visible_count_f32(&self) -> f32 {
        self.visible_count as f32
    }

    /// Return the integer range of snapshot indices needed for rendering.
    /// Includes one extra snapshot on each side to cover partially visible columns.
    pub fn visible_range(&self) -> (usize, usize) {
        let left = self.scroll_left();
        let start = (left.floor() as isize).max(0) as usize;
        let end = (self.scroll_right.ceil() as usize).min(self.timeline.snapshots.len());
        if start >= end {
            return (0, 0);
        }
        (start, end)
    }

    /// Return the visible slice of snapshots (integer-aligned for data access).
    pub fn visible_snapshots(&self) -> &[DepthTimelineEntry] {
        let (start, end) = self.visible_range();
        &self.timeline.snapshots[start..end]
    }

    /// Pan by a fractional number of columns (positive = pan right / show later data).
    /// Allows scrolling past both ends by up to `visible_count` columns of empty space.
    pub fn pan_x(&mut self, delta_cols: f32) {
        let max_right = self.timeline.snapshots.len() as f32 + self.visible_count as f32;
        self.scroll_right = (self.scroll_right + delta_cols).clamp(0.0, max_right);
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
