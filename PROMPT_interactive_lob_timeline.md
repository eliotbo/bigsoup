# Task: Interactive LOB Timeline Window in Vizza

## Goal

Add a `DepthTimelineWindow` to the `vizza` crate that renders the LOB depth timeline
interactively in a native window (winit + wgpu) instead of (or in addition to) saving a
static PNG. Update `lob_timeline.rs` to open this window after the simulation finishes,
showing the full tick history and allowing the user to pan and zoom.

---

## Context: what already exists

### Data model — `depth_timeline.rs`

`DepthTimelineState` is the complete view model. It already has everything needed:

```rust
pub struct DepthTimelineState {
    pub timeline: DepthTimeline,        // all snapshots
    pub visible_right: usize,           // rightmost visible column (exclusive)
    pub visible_count: usize,           // how many columns fit on screen
    pub price_min: f32,
    pub price_max: f32,
    pub auto_y_scale: bool,
    pub column_width_px: f32,
}

impl DepthTimelineState {
    pub fn visible_snapshots(&self) -> &[DepthTimelineEntry] { ... }
    pub fn pan_x(&mut self, delta_cols: i32) { ... }   // already implemented
    pub fn auto_scale_y(&mut self) { ... }              // already implemented
}
```

### Headless renderer — `depth_timeline_renderer.rs` (`DepthTimelineRenderer`)

- Creates a **headless** wgpu device (no surface, `compatible_surface: None`)
- Calls `prepare_instances(state)` to build `Vec<DepthTimelineInstance>` + `DepthTimelineUniform`
- Renders into an offscreen `Rgba8UnormSrgb` texture
- Reads pixels back via a staging buffer
- Draws labels and grid lines **CPU-side** using a tiny 4×6 bitmap font into the pixel buffer
- Encodes to PNG via the `png` crate

The GPU shader is `shaders/depth_timeline.wgsl`. Each instance is one (column, price_level)
pair: `{ column_index, price, log_quantity, color_r, color_g, color_b }`. The uniform block
holds the viewport transform: price range, column range, `max_log_qty`, window size,
`column_width_px`, `margin_left`, `margin_bottom`.

### Interactive depth window — `depth_window.rs` (`DepthWindow`)

- Creates a **real winit window** with its own wgpu device + swapchain surface
- Uses `shaders/depth_bar.wgsl` to show a **single LOB snapshot** (one tick) as a vertical
  strip of horizontal bars
- Has `update_snapshot(&DepthSnapshot)`, `render()`, `resize()`, `request_redraw()`
- Plugs into `app.rs`'s event loop via `window_id()` matching

`DepthWindow` is the structural template to follow for `DepthTimelineWindow`.

### Text rendering

`DepthTimelineRenderer` uses a hand-rolled 4×6 bitmap font (CPU pixel writes). The rest of
vizza uses **Glyphon** (already in `Cargo.toml`) for GPU text. For the interactive window,
use Glyphon for price and tick labels — the `GridRenderer` in `view/grid.rs` is the
reference for how Glyphon is set up.

---

## What to build

### 1. `crates/vizza/src/depth_timeline_window.rs` — `DepthTimelineWindow`

Modelled on `DepthWindow`, but for the timeline view. Key differences:

**Setup**
- Own winit window, wgpu instance, adapter, device, queue, surface — same pattern as
  `DepthWindow::new()`
- Surface format: prefer sRGB (same as `DepthWindow`)
- Compile `shaders/depth_timeline.wgsl` (the same shader used by `DepthTimelineRenderer`)
- Uniform buffer, bind group, instance buffer — same layout as `DepthTimelineRenderer`
  (reuse `DepthTimelineUniform` and `DepthTimelineInstance`; make them `pub(crate)` or move
  them to `depth_timeline.rs`)
- One Glyphon `TextRenderer` + `TextAtlas` + `SwashCache` for labels

**`update(&mut self, state: &DepthTimelineState)`**
- Calls the same `prepare_instances` logic as `DepthTimelineRenderer::prepare_instances`
  (extract that method to a free function or make it shared)
- Uploads uniform + instance data to GPU buffers
- Sets `needs_redraw = true`

**`render(&mut self)`**
- Guard with `needs_redraw`
- `surface.get_current_texture()` → render pass
- Draw instances with the depth timeline pipeline
- Draw Glyphon text (price labels on Y axis, tick numbers on X axis) in a second pass
  using `TextRenderer::render()`
- `output.present()`

**`resize(&mut self, size)`** — reconfigure surface, update uniform `window_w/window_h`,
update Glyphon viewport

**`window_id()`** — returns `self.window.id()` for event dispatch

**`request_redraw()`** — `self.window.request_redraw()`

### 2. Input handling inside `DepthTimelineWindow`

Handle these winit events (called from the event loop in `lob_timeline.rs`):

| Event | Action |
|-------|--------|
| `MouseInput` left button down | begin drag, record `drag_start_x` |
| `CursorMoved` while dragging | `delta_px = drag_start_x - cursor_x`; `delta_cols = (delta_px / column_width_px) as i32`; call `state.pan_x(delta_cols)`; update `drag_start_x` |
| `MouseWheel` scroll Y | zoom: `column_width_px *= 1.1^delta`; clamp to `[2.0, 200.0]`; recompute `visible_count = (window_w - margin_left) / column_width_px`; call `state.auto_scale_y()` if `auto_y_scale` |
| `KeyboardInput` `Home` | jump to beginning: `state.visible_right = state.visible_count` |
| `KeyboardInput` `End` | jump to end: `state.visible_right = state.timeline.snapshots.len()` |

The `DepthTimelineState` is passed in by mutable reference at each event and `update` call —
the window does not own the state.

### 3. Event loop in `lob_timeline.rs`

Replace the current pattern of:
```rust
renderer.render_to_png(&state, &args.output)?;
```

with opening an interactive window. The simulation has already run and all snapshots are in
`state` by the time the window opens, so no threading is needed — it is a post-hoc viewer.

Use `winit::event_loop::EventLoop::new()` + `EventLoop::run_app()`. The app struct holds:
- `Option<DepthTimelineWindow>` (created on `resumed`)
- `DepthTimelineState` (moved in)

In `ApplicationHandler`:
- `resumed`: create `DepthTimelineWindow`
- `window_event` dispatches resize / keyboard / mouse events to the window, then calls
  `window.update(&state)` and `window.render()`
- `about_to_wait`: `window.request_redraw()`

Keep the `--output` flag working: if provided, also call `render_to_png` before opening the
window (or skip the window with a `--no-window` flag).

### 4. Export to `lib.rs`

Add to `lib.rs`:
```rust
pub mod depth_timeline_window;
pub use depth_timeline_window::DepthTimelineWindow;
```

---

## Constraints and style notes

- Follow the existing patterns in `depth_window.rs` exactly — same error handling (`anyhow`),
  same `pollster::block_on` for async wgpu calls, same `bytemuck` for buffer uploads
- `DepthTimelineInstance` and `DepthTimelineUniform` are currently private in
  `depth_timeline_renderer.rs`; move them to `depth_timeline.rs` (or make them
  `pub(crate)`) so both the renderer and the window can share them without duplication
- The `prepare_instances` logic should also be extracted to a shared free function in
  `depth_timeline.rs` for the same reason
- Do not add any new dependencies — winit, wgpu, bytemuck, glyphon, anyhow, pollster are
  all already in `crates/vizza/Cargo.toml`
- The window title should be `"LOB Timeline"` with the tick range shown, e.g.
  `"LOB Timeline — ticks 1–1000"`
- Default window size: 1400×800 (matching the current PNG defaults in `lob_timeline.rs`)
