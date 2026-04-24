// Grid shader for hairline rendering
// Renders 1px lines with per-vertex colors

struct VertexInput {
    @location(0) position: vec2<f32>,  // Viewport-relative coordinates (0.0 to 1.0)
    @location(1) color: vec4<f32>,     // RGBA color with alpha
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) screen_pos: vec2<f32>,
}

struct Uniforms {
    viewport: vec4<f32>,  // x, y, w, h
    window: vec4<f32>,    // w, h, _pad0, _pad1
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Extract viewport and window dimensions
    let viewport_x = uniforms.viewport.x;
    let viewport_y = uniforms.viewport.y;
    let viewport_w = uniforms.viewport.z;
    let viewport_h = uniforms.viewport.w;
    let window_w = uniforms.window.x;
    let window_h = uniforms.window.y;

    // Convert viewport-relative (0.0 to 1.0) to viewport pixel coordinates
    let x_px_in_viewport = in.position.x * viewport_w;
    let y_px_in_viewport = in.position.y * viewport_h;

    // Add viewport offset to get window pixel coordinates
    let x_px_window = x_px_in_viewport + viewport_x;
    let y_px_window = y_px_in_viewport + viewport_y;

    // Convert window pixels to NDC
    let x_ndc = (x_px_window / window_w) * 2.0 - 1.0;
    let y_ndc = 1.0 - (y_px_window / window_h) * 2.0;

    out.clip_position = vec4<f32>(x_ndc, y_ndc, 0.0, 1.0);
    out.color = in.color;
    out.screen_pos = vec2<f32>(x_px_window, y_px_window);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Detect dividend lines (green color: high green, low red/blue)
    let is_dividend = in.color.g > 0.9 && in.color.r < 0.1 && in.color.b < 0.1;

    if (is_dividend) {
        // Create dashed pattern using y-coordinate
        // Dash length: 8px, Gap length: 6px
        let dash_cycle = 14.0; // 8 + 6
        let dash_length = 8.0;
        let y_mod = in.screen_pos.y % dash_cycle;

        // Discard pixels in the gap
        if (y_mod > dash_length) {
            discard;
        }
    }

    return in.color;
}
