// Depth histogram shader — draws horizontal bars for bid/ask levels.
//
// Each instance is one price level. The vertex shader generates a quad
// (6 vertices, 2 triangles) per instance.
//
// All bars grow from the left edge rightward.
// Bars are laid out top-to-bottom by slot index with exact pixel sizing.

struct DepthUniform {
    max_qty: f32,
    window_w: f32,
    window_h: f32,
    bar_height_px: f32,  // exact height of each bar in pixels
    gap_px: f32,         // exact gap between bars in pixels
    margin_left: f32,    // left margin in pixels for Y-axis labels
    _pad0: f32,
    _pad1: f32,
}

@group(0) @binding(0)
var<uniform> u: DepthUniform;

struct Instance {
    @location(0) slot_index: f32,  // 0-based row from the top
    @location(1) quantity: f32,
    @location(2) side: f32,        // 0 = bid, 1 = ask (for colouring)
    @location(3) color_r: f32,
    @location(4) color_g: f32,
    @location(5) color_b: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    inst: Instance,
) -> VertexOutput {
    var out: VertexOutput;
    out.color = vec3<f32>(inst.color_r, inst.color_g, inst.color_b);

    if u.max_qty <= 0.0 {
        out.clip_position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
        return out;
    }

    // Y position: each slot is bar_height + gap pixels, starting from top
    let slot = inst.slot_index;
    let y_top = slot * (u.bar_height_px + u.gap_px);
    let y_bot = y_top + u.bar_height_px;

    // X: normalised quantity, bar spans from margin to right edge at max
    let qty_t = inst.quantity / u.max_qty;
    let bar_area_w = u.window_w - u.margin_left;
    let x_right = u.margin_left + qty_t * bar_area_w;

    // 6 vertices forming a quad (two triangles)
    var px: f32;
    var py: f32;
    switch vi {
        case 0u: { px = u.margin_left; py = y_top; }
        case 1u: { px = u.margin_left; py = y_bot; }
        case 2u: { px = x_right;      py = y_top; }
        case 3u: { px = x_right;      py = y_top; }
        case 4u: { px = u.margin_left; py = y_bot; }
        case 5u: { px = x_right;      py = y_bot; }
        default: { px = 0.0; py = 0.0; }
    }

    // Pixel coords → clip space
    let x_clip = (px / u.window_w) * 2.0 - 1.0;
    let y_clip = 1.0 - (py / u.window_h) * 2.0;

    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
