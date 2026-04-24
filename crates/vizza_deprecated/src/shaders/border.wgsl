// Simple border shader to visualize viewport bounds

struct BorderUniform {
    viewport_x: f32,
    viewport_y: f32,
    viewport_w: f32,
    viewport_h: f32,
    window_w: f32,
    window_h: f32,
    border_width: f32,
    _padding: f32,
    border_color: vec3<f32>,
    _padding2: f32,
}

@group(0) @binding(0)
var<uniform> border: BorderUniform;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_idx: u32) -> VertexOutput {
    var out: VertexOutput;

    // Generate 8 vertices for 4 border lines (2 vertices per line)
    // Top, Right, Bottom, Left
    var positions = array<vec2<f32>, 8>(
        // Top border
        vec2<f32>(0.0, 0.0),
        vec2<f32>(border.viewport_w, 0.0),
        // Right border
        vec2<f32>(border.viewport_w, 0.0),
        vec2<f32>(border.viewport_w, border.viewport_h),
        // Bottom border
        vec2<f32>(border.viewport_w, border.viewport_h),
        vec2<f32>(0.0, border.viewport_h),
        // Left border
        vec2<f32>(0.0, border.viewport_h),
        vec2<f32>(0.0, 0.0),
    );

    let pos = positions[vertex_idx];

    // Convert to window pixel coordinates
    let x_px = pos.x + border.viewport_x;
    let y_px = pos.y + border.viewport_y;

    // Convert to clip space
    let x_clip = (x_px / border.window_w) * 2.0 - 1.0;
    let y_clip = 1.0 - (y_px / border.window_h) * 2.0;

    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);
    out.color = border.border_color;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
