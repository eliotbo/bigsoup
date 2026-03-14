struct ViewUniform {
    affine: vec4<f32>,
    viewport: vec4<f32>,
    window: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> view: ViewUniform;

struct MarkerInstance {
    @location(0) tick_idx_rel: f32,
    @location(1) price_norm: f32,
    @location(2) radius_px: f32,
    @location(3) _pad: f32,
    @location(4) color: vec4<f32>,
}

struct MarkerVertex {
    @location(5) position: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(vertex: MarkerVertex, instance: MarkerInstance) -> VertexOutput {
    var out: VertexOutput;

    let sx = view.affine.x;
    let bx = view.affine.y;
    let sy = view.affine.z;
    let by = view.affine.w;

    let viewport_x = view.viewport.x;
    let viewport_y = view.viewport.y;
    let viewport_w = view.viewport.z;
    let viewport_h = view.viewport.w;

    let window_w = view.window.x;
    let window_h = view.window.y;

    let x_center_viewport = (instance.tick_idx_rel * sx) + bx;
    let y_center_viewport = (instance.price_norm * sy) + by;

    let x_px_in_viewport = (x_center_viewport + 1.0) * 0.5 * viewport_w;
    let y_px_in_viewport = (1.0 - y_center_viewport) * 0.5 * viewport_h;

    let x_px_window = x_px_in_viewport + viewport_x + vertex.position.x * instance.radius_px / 2.0;
    let y_px_window = y_px_in_viewport + viewport_y + vertex.position.y * instance.radius_px / 2.0;

    let x_clip = (x_px_window / window_w) * 2.0 - 1.0;
    let y_clip = 1.0 - (y_px_window / window_h) * 2.0;

    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);
    out.uv = vertex.position;
    out.color = instance.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dist = length(in.uv);
    if (dist > 1.0) {
        discard;
    }

    let alpha = in.color.w * smoothstep(1.0, 0.8, dist);
    return vec4<f32>(in.color.xyz, alpha);
}
