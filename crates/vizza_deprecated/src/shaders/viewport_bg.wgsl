// Viewport background shader - draws a background quad in the viewport area

struct ViewportBgUniform {
    viewport_x: f32,
    viewport_y: f32,
    viewport_w: f32,
    viewport_h: f32,
    window_w: f32,
    window_h: f32,
    bg_color_r: f32,
    bg_color_g: f32,
    bg_color_b: f32,
    _pad0: f32,
}

@group(0) @binding(0)
var<uniform> viewport_bg: ViewportBgUniform;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_idx: u32) -> VertexOutput {
    var out: VertexOutput;

    // Generate 6 vertices for 2 triangles forming a quad
    // Triangle 1: top-left, bottom-left, top-right
    // Triangle 2: top-right, bottom-left, bottom-right
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),                                    // top-left
        vec2<f32>(0.0, viewport_bg.viewport_h),                 // bottom-left
        vec2<f32>(viewport_bg.viewport_w, 0.0),                 // top-right
        vec2<f32>(viewport_bg.viewport_w, 0.0),                 // top-right
        vec2<f32>(0.0, viewport_bg.viewport_h),                 // bottom-left
        vec2<f32>(viewport_bg.viewport_w, viewport_bg.viewport_h), // bottom-right
    );

    let pos = positions[vertex_idx];

    // Convert to window pixel coordinates
    let x_px = pos.x + viewport_bg.viewport_x;
    let y_px = pos.y + viewport_bg.viewport_y;

    // Convert to clip space
    let x_clip = (x_px / viewport_bg.window_w) * 2.0 - 1.0;
    let y_clip = 1.0 - (y_px / viewport_bg.window_h) * 2.0;

    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Background color from theme
    return vec4<f32>(viewport_bg.bg_color_r, viewport_bg.bg_color_g, viewport_bg.bg_color_b, 1.0);
}
