struct ViewUniform {
    affine: vec4<f32>,   // sx, bx, sy, by
    viewport: vec4<f32>, // x, y, w, h
    window: vec4<f32>,   // w, h, _, _
}

@group(0) @binding(0)
var<uniform> view: ViewUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
}

struct InstanceInput {
    @location(1) x_start: f32,
    @location(2) x_end: f32,
    @location(3) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(vertex: VertexInput, instance: InstanceInput) -> VertexOutput {
    let sx = view.affine.x;
    let bx = view.affine.y;

    let viewport_x = view.viewport.x;
    let viewport_y = view.viewport.y;
    let viewport_w = view.viewport.z;
    let viewport_h = view.viewport.w;

    let window_w = view.window.x;
    let window_h = view.window.y;

    // Interpolate between the overlay bounds using the quad vertex position.
    let t = (vertex.position.x + 1.0) * 0.5;
    let x_viewport = ((mix(instance.x_start, instance.x_end, t) * sx) + bx);
    let y_viewport = vertex.position.y;

    let x_px_in_viewport = (x_viewport + 1.0) * 0.5 * viewport_w;
    let y_px_in_viewport = (1.0 - y_viewport) * 0.5 * viewport_h;

    let x_px_window = x_px_in_viewport + viewport_x;
    let y_px_window = y_px_in_viewport + viewport_y;

    let x_clip = (x_px_window / window_w) * 2.0 - 1.0;
    let y_clip = 1.0 - (y_px_window / window_h) * 2.0;

    var out: VertexOutput;
    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);
    out.color = instance.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
