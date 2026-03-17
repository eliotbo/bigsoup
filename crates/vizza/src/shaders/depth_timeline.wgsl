// Depth timeline shader — draws horizontal bars for bid/ask levels across time columns.
//
// Each instance is one (column, price_level) pair with nonzero depth.
// The vertex shader generates a quad (6 vertices, 2 triangles) per instance.
// Fragments outside the chart viewport are discarded.

struct Uniform {
    price_min: f32,
    price_max: f32,
    col_start: f32,      // first visible column index
    col_count: f32,      // number of visible columns
    max_log_qty: f32,    // max log10(1+qty) across all visible data
    window_w: f32,
    window_h: f32,
    column_width_px: f32,
    margin_left: f32,    // left margin
    margin_bottom: f32,  // bottom margin
    margin_top: f32,     // top margin
    margin_right: f32,   // right margin
}

@group(0) @binding(0)
var<uniform> u: Uniform;

struct Instance {
    @location(0) column_index: f32,   // absolute column index
    @location(1) price: f32,          // price in dollars
    @location(2) log_quantity: f32,   // log10(1 + quantity)
    @location(3) color_r: f32,
    @location(4) color_g: f32,
    @location(5) color_b: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) pixel_pos: vec2<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    inst: Instance,
) -> VertexOutput {
    var out: VertexOutput;
    out.color = vec3<f32>(inst.color_r, inst.color_g, inst.color_b);

    if u.max_log_qty <= 0.0 || u.col_count <= 0.0 {
        out.clip_position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
        out.pixel_pos = vec2<f32>(0.0, 0.0);
        return out;
    }

    let price_range = u.price_max - u.price_min;
    if price_range <= 0.0 {
        out.clip_position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
        out.pixel_pos = vec2<f32>(0.0, 0.0);
        return out;
    }

    // Chart area (excluding margins)
    let chart_left = u.margin_left;
    let chart_top = u.margin_top;
    let chart_width = u.window_w - u.margin_left - u.margin_right;
    let chart_height = u.window_h - u.margin_top - u.margin_bottom;

    // X: column position within chart area (with gap between columns)
    let col_gap = 3.0;
    let rel_col = inst.column_index - u.col_start;
    let col_left_px = chart_left + rel_col * u.column_width_px;
    let usable_col_width = u.column_width_px - col_gap;

    // Bar width within the column, proportional to log_quantity
    let bar_width_px = (inst.log_quantity / u.max_log_qty) * usable_col_width;

    // Y: price -> pixel (higher price = lower Y in pixel coords, but we want higher price at top)
    // price_t = 0 at price_min (bottom), 1 at price_max (top)
    let price_t = (inst.price - u.price_min) / price_range;

    // Each $0.01 tick gets some pixel height. Calculate pixels per dollar.
    // Subtract 2px gap between adjacent price level bars.
    let px_per_dollar = chart_height / price_range;
    let bar_gap = 2.0;
    let raw_bar_height = px_per_dollar * 0.01 * 0.5;
    let bar_height_px = max(raw_bar_height - bar_gap, 1.0);

    // Y center in pixel coords (0 = top of window)
    let y_center_px = chart_top + chart_height * (1.0 - price_t);

    let y_top = y_center_px - bar_height_px * 0.5;
    let y_bot = y_center_px + bar_height_px * 0.5;
    let x_left = col_left_px;
    let x_right = col_left_px + bar_width_px;

    // 6 vertices forming a quad (two triangles)
    var px: f32;
    var py: f32;
    switch vi {
        case 0u: { px = x_left;  py = y_top; }
        case 1u: { px = x_left;  py = y_bot; }
        case 2u: { px = x_right; py = y_top; }
        case 3u: { px = x_right; py = y_top; }
        case 4u: { px = x_left;  py = y_bot; }
        case 5u: { px = x_right; py = y_bot; }
        default: { px = 0.0; py = 0.0; }
    }

    out.pixel_pos = vec2<f32>(px, py);

    // Pixel coords -> clip space
    let x_clip = (px / u.window_w) * 2.0 - 1.0;
    let y_clip = 1.0 - (py / u.window_h) * 2.0;

    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Discard fragments outside the chart viewport
    let chart_left = u.margin_left;
    let chart_top = u.margin_top;
    let chart_right = u.window_w - u.margin_right;
    let chart_bottom = u.window_h - u.margin_bottom;

    if in.pixel_pos.x < chart_left || in.pixel_pos.x > chart_right ||
       in.pixel_pos.y < chart_top  || in.pixel_pos.y > chart_bottom {
        discard;
    }

    return vec4<f32>(in.color, 0.10);
}
