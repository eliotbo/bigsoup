# Depth Chart Timeline — Implementation Prompt

## Goal

Build a new graph type in the `vizza` crate that renders a **horizontal timeline of LOB (limit order book) depth histograms**. Each time tick on the X axis contains a full sideways depth histogram (bars extending rightward from each price level, colored green for bids and red for asks). The user can pan left/right through time, and the Y axis (price) auto-scales to fit the visible data.

This is a new window and a new renderer — it does **not** go inside the existing candlestick viewports. It reuses vizza's existing grid, axis, event handling, and theme infrastructure.

---

## Data Model

### Input: `DepthTimeline`

```rust
pub struct DepthTimeline {
    /// Snapshots in chronological order, taken every N simulation ticks.
    pub snapshots: Vec<DepthTimelineEntry>,
}

pub struct DepthTimelineEntry {
    /// Simulation tick or Unix timestamp for the X axis.
    pub tick: u64,
    /// Bid levels: Vec<(price, quantity)> sorted descending by price (best bid first).
    pub bids: Vec<(f32, f32)>,
    /// Ask levels: Vec<(price, quantity)> sorted ascending by price (best ask first).
    pub asks: Vec<(f32, f32)>,
}
```

Each snapshot is rendered as a vertical column of horizontal bars at that X position. The bar width (horizontal extent) is `log10(1 + quantity)`, normalized so the widest bar across all visible snapshots fills the allocated column width. Bars are colored green (bids) and red (asks) using the existing `ColorPalette` (`candle_up_market` / `candle_down_market`).

The price tick size is **$0.01**. Each $0.01 level gets one row. Empty levels (including the spread) are visible gaps with no bar.

---

## Architecture — What to Reuse from Vizza

The existing vizza crate has a clean MVC architecture. Study and reuse these components:

### Grid and Axis System
- **`crates/vizza/src/view/grid.rs`** — `GridRenderer` handles grid lines and text labels via Glyphon. It has `update_grid_lines()` which generates price labels on the Y axis and time labels on the X axis. Reuse this directly or adapt it.
- **`crates/vizza/src/price_spacing.rs`** — `PriceTickSpacing` and `select_price_spacing()` select nice price intervals and generate tick values. Use this for the Y axis.
- **`crates/vizza/src/time_spacing.rs`** — `TimeTickSpacing` and `select_time_spacing()` do the same for time. Use this for the X axis if using Unix timestamps, or write a simpler tick-based spacing if using simulation ticks.

### Pan, Scroll, Zoom Interaction
- **`crates/vizza/src/event.rs`** — `MouseState` and `dispatch_window_event()` handle click-drag panning, scroll zooming, and keyboard shortcuts. The pan logic in `handle_cursor_moved()` (throttled to 30ms) calls `renderer.handle_pan_viewport()`.
- **`crates/vizza/src/state/viewport_state.rs`** — `ViewportState` tracks `viewport_right_ts`, `pan_offset_y`, `fixed_y_min/max`, and `view_settings.auto_y_scale`. Its `handle_pan()` method handles both X panning and Y rescaling.
- The existing keyboard handler (`handle_keyboard_input()` in `event.rs`) already maps `Y` to toggle `auto_y_scale` — reuse this.

### Rendering Pattern
- **`crates/vizza/src/view/bars.rs`** — `BarRenderer` is the model for instanced GPU rendering. It uses a `ViewUniform` for the viewport transform, an instance buffer with per-bar data, and a WGSL shader. Follow this pattern for the depth columns.
- **`crates/vizza/src/view/viewport.rs`** — `ViewportRenderer` draws the viewport background and border. Reuse directly.
- **`crates/vizza/src/view/viewport_view.rs`** — `ViewportView` wires together all renderers (bars, grid, overlays) for one viewport.

### Headless Screenshot for Validation
- **`crates/vizza/src/depth_renderer.rs`** — `DepthRenderer` shows the pattern for headless GPU rendering: create a device with no surface, render to an offscreen texture, `copy_texture_to_buffer`, map-read the pixels, and save as PNG via the `png` crate. **You must use this approach to take screenshots and verify your work visually.**

### Config and Theme
- **`crates/vizza/src/config.rs`** — `Config`, `ColorPalette`, `Theme`. Use `ColorPalette::from_theme()` for colors.

---

## Rendering Design

### X Axis (Time)
- Each depth snapshot gets a fixed-width column on the X axis (e.g. 7-10 pixels per snapshot, configurable).
- Panning left/right scrolls through time. Click-drag with the left mouse button pans.
- The X axis labels show tick numbers or timestamps.

### Y Axis (Price)
- Linear price axis with $0.01 resolution.
- Each pixel row maps to a price. The mapping is: `y_pixel = (price_max - price) / (price_max - price_min) * viewport_height`.
- **Auto Y-scaling (default):** Scan all visible snapshots, find the lowest bid price and highest ask price across all of them, add 5% margin, and set `price_min`/`price_max` so these span 90% of the viewport height.
- **Press `Y` to toggle auto-scaling off.** When off, the Y range is fixed and the user can drag vertically to pan the price axis (same as the existing candlestick behavior).

### Bar Rendering
- Within each time column, for each price level that has depth:
  - Draw a horizontal bar starting from the column's left edge, extending rightward.
  - Bar width = `log10(1 + quantity) / max_log_qty * column_width` where `max_log_qty` is the max across all visible snapshots.
  - Bar height = 1 pixel per $0.01 tick (or whatever the Y-axis zoom maps to).
  - Color: green (`candle_up_market`) for bids, red (`candle_down_market`) for asks.
- The spread (price levels between best bid and best ask with no orders) should be visible as a gap.

### GPU Approach
Use instanced rendering like `BarRenderer`:
- One instance per (snapshot_index, price_level) pair that has nonzero depth.
- Instance data: `{ column_index: f32, price: f32, log_quantity: f32, side: f32, color: [f32; 3] }`.
- The vertex shader generates a quad per instance, positioned by column_index (X) and price (Y), with width proportional to log_quantity.
- A `ViewUniform` maps the visible time range and price range to clip space.

### Grid Lines
- Horizontal grid lines at nice price intervals (use `select_price_spacing()`).
- Vertical grid lines at regular time intervals.
- Price labels on the right or left edge.
- Time labels on the bottom.

---

## Implementation Steps

1. **Create `DepthTimelineState`** — Analogous to `ViewportState`. Holds: the `DepthTimeline` data, visible time range (left/right column indices), visible price range (min/max), auto_y_scale flag, pan state.

2. **Create `DepthTimelineRenderer`** — Analogous to `BarRenderer`. WGSL shader + instanced pipeline. Takes the visible slice of snapshots, builds instance data, uploads to GPU.

3. **Create the WGSL shader** — `depth_timeline.wgsl`. Vertex shader maps (column_index, price, log_quantity) → clip space quad. Fragment shader outputs the color.

4. **Create `DepthTimelineView`** — Analogous to `ViewportView`. Wires together `DepthTimelineRenderer` + `GridRenderer` + `ViewportRenderer`.

5. **Create `DepthTimelineWindow`** — New winit window (follow the pattern in `depth_window.rs` and `app.rs` for multi-window support). Handles its own resize, redraw, and input events.

6. **Create `DepthTimelineHeadlessRenderer`** — Follow `depth_renderer.rs` pattern for offscreen rendering to PNG. Use this to validate the output visually.

7. **Create a test binary** (`src/bin/depth_timeline_screenshot.rs`) that generates synthetic data and saves a screenshot. The synthetic data should have:
   - 20+ time snapshots
   - Price around $100.00 with $0.01 ticks
   - Spread of ~5-10 ticks
   - 30-50 bid levels and 30-50 ask levels per snapshot
   - Quantities varying over time (some levels appearing/disappearing)

8. **Take a screenshot, look at it, and iterate** until the graph looks good. Specifically verify:
   - The spread is clearly visible as a gap between green and red
   - Price labels are readable and not overlapping
   - Time labels are visible on the X axis
   - The bars are distinguishable at the column width chosen
   - The auto Y-scaling correctly frames the data with ~5% margin
   - Panning range makes sense (test by varying the visible window)

---

## Quality Checklist

Before considering the task complete, take a screenshot and verify:

- [ ] Green bid bars and red ask bars are clearly distinguishable
- [ ] The spread gap is visible between bids and asks at every time tick
- [ ] Y-axis price labels are present, readable, and at sensible intervals (not every $0.01)
- [ ] X-axis time/tick labels are present and readable
- [ ] Grid lines are visible but subtle (not distracting)
- [ ] The graph fills the viewport well (90% height usage)
- [ ] Log scaling makes small quantities visible while large quantities don't dominate
- [ ] The background matches the vizza theme (dark background)
- [ ] Bars within a column don't overlap with neighboring columns

---

## Key Files Reference

| File | Purpose |
|------|---------|
| `crates/vizza/src/view/bars.rs` | Instanced bar rendering pattern (BarRenderer) |
| `crates/vizza/src/view/grid.rs` | Grid lines + Glyphon text labels (GridRenderer) |
| `crates/vizza/src/state/viewport_state.rs` | Viewport state: pan, zoom, y-range (ViewportState) |
| `crates/vizza/src/event.rs` | Mouse/keyboard event handling (MouseState) |
| `crates/vizza/src/price_spacing.rs` | Price axis tick selection |
| `crates/vizza/src/time_spacing.rs` | Time axis tick selection |
| `crates/vizza/src/view/viewport.rs` | Viewport background/border renderer |
| `crates/vizza/src/view/viewport_view.rs` | Wires all renderers together per viewport |
| `crates/vizza/src/depth_renderer.rs` | Headless GPU rendering + PNG screenshot pattern |
| `crates/vizza/src/depth_snapshot.rs` | DepthSnapshot struct (bids/asks as Vec<(f32,f32)>) |
| `crates/vizza/src/shaders/depth_bar.wgsl` | Existing depth bar shader (simple version) |
| `crates/vizza/src/shaders/bar.wgsl` | Candlestick shader (complex instanced example) |
| `crates/vizza/src/config.rs` | Config, ColorPalette, Theme |
| `crates/vizza/src/app.rs` | Multi-window winit event loop |
| `crates/vizza/src/renderer.rs` | Main renderer with dirty tracking, framebuffer |
| `CLAUDE.md` | Project instructions — **never search /media/data10t** |

---

## Constraints

- **Do not modify the existing candlestick rendering** — this is a new, separate graph type.
- **Do not search or read from `/media/data10t`** — this is a large data drive unrelated to this task.
- **Use the existing `ColorPalette` and `Theme`** — do not invent new color schemes.
- **Take screenshots and verify visually** — use the headless `render_to_png()` pattern. Read the PNG file to view it. Iterate until it looks good.
- **Price tick size is $0.01** — one row per cent. The Y axis must reflect this.
- **Log10 scaling for bar widths** — use `log10(1 + quantity)` so small depths are visible.
- **The graph must compile** — run `cargo check` after each significant change.
