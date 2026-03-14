// View uniforms for camera-local affine mapping (std140 layout)
struct ViewUniform {
    affine: vec4<f32>,   // sx, bx, sy, by
    viewport: vec4<f32>, // x, y, w, h
    window: vec4<f32>,   // w, h, bar_width_px, volume_enabled_flag
    candle_up_market: vec4<f32>,    // RGB + padding
    candle_up_offhours: vec4<f32>,  // RGB + padding
    candle_down_market: vec4<f32>,  // RGB + padding
    candle_down_offhours: vec4<f32>,// RGB + padding
    wick_color: vec4<f32>,          // RGB + padding
    volume_color: vec4<f32>,        // RGB + padding
}

@group(0) @binding(0)
var<uniform> view: ViewUniform;

// Per-instance data
struct Instance {
    @location(0) tick_idx_rel: f32,  // Relative to t0_idx
    @location(1) open: f32,           // Body bottom
    @location(2) close: f32,          // Body top
    @location(3) high: f32,           // Top of upper wick
    @location(4) low: f32,            // Bottom of lower wick
    @location(5) flags: u32,
    @location(8) volume_norm: f32,
}

// Unit quad vertices (for bar body) or line vertices (for wick)
struct VertexInput {
    @location(6) position: vec2<f32>,  // Unit quad: [-0.5, -0.5] to [0.5, 0.5], or line vertex
    @location(7) geometry_kind: f32,   // 0=body, 1=wick, 2=volume
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) base_color: vec3<f32>,
    @location(1) wick_params: vec3<f32>, // x = is_wick flag, y = body_low, z = body_high
    @location(2) wick_price: f32,
    @location(3) volume_flag: f32,
}

@vertex
fn vs_main(
    vertex: VertexInput,
    instance: Instance,
) -> VertexOutput {
    var out: VertexOutput;

    // Extract affine components
    let sx = view.affine.x;
    let bx = view.affine.y;
    let sy = view.affine.z;
    let by = view.affine.w;

    // Extract viewport components
    let viewport_x = view.viewport.x;
    let viewport_y = view.viewport.y;
    let viewport_w = view.viewport.z;
    let viewport_h = view.viewport.w;

    // Extract window components
    let window_w = view.window.x;
    let window_h = view.window.y;
    let bar_width_px = view.window.z;

    let geometry_kind = vertex.geometry_kind;
    let is_wick = geometry_kind > 0.5 && geometry_kind < 1.5;
    let is_volume = geometry_kind > 1.5;
    let volume_enabled = view.window.w > 0.5;

    // Get x center in viewport-local clip space
    let x_center_viewport = (instance.tick_idx_rel * sx) + bx;

    let body_low = min(instance.open, instance.close);
    let body_high = max(instance.open, instance.close);

    var wick_flag = 0.0;
    var wick_price = 0.0;
    var volume_flag = 0.0;

    // Convert x center to viewport pixels early to ensure pixel alignment
    let x_center_px_in_viewport = (x_center_viewport + 1.0) * 0.5 * viewport_w;

    // Round to nearest pixel for consistent alignment
    let x_center_px_aligned = floor(x_center_px_in_viewport + 0.5);

    var x_px_in_viewport: f32;
    var y_px_in_viewport: f32;

    if (is_wick) {
        // Wick rendering: thin vertical line from low to high
        let y_center_viewport = ((instance.low + instance.high) * 0.5 * sy) + by;
        let half_height_viewport = (instance.high - instance.low) * 0.5 * sy;

        // Wick width in pixels (1/3 of bar width, minimum 1px)
        var wick_width_px = max(1.0, floor(bar_width_px / 3.0));
        let half_wick_width_px = wick_width_px * 0.5;

        // Calculate x position in pixels with pixel alignment
        x_px_in_viewport = x_center_px_aligned + vertex.position.x * wick_width_px;

        // Calculate y in clip space then convert to pixels
        let y_normalized = vertex.position.y;
        let y_viewport = y_center_viewport + y_normalized * half_height_viewport;
        y_px_in_viewport = (1.0 - y_viewport) * 0.5 * viewport_h;

        // Track wick details for fragment-stage blending
        wick_flag = 1.0;
        let t = (y_normalized + 1.0) * 0.5;
        wick_price = instance.low + (instance.high - instance.low) * t;
    } else if (is_volume) {
        // Volume bars anchored to bottom of viewport
        // Use exact bar width in pixels
        let half_bar_width_px = bar_width_px * 0.5;

        // Calculate x position in pixels with pixel alignment
        x_px_in_viewport = x_center_px_aligned + vertex.position.x * bar_width_px;

        let y_bottom = -1.0;
        let band_fraction = 0.25;
        let draw_volume = volume_enabled && instance.volume_norm > 0.0;
        var effective_volume = 0.0;
        if (draw_volume) {
            effective_volume = instance.volume_norm;
        }
        let y_top = y_bottom + band_fraction * 2.0 * effective_volume;
        let t = clamp(vertex.position.y + 0.5, 0.0, 1.0);
        let y_viewport = mix(y_bottom, y_top, t);
        y_px_in_viewport = (1.0 - y_viewport) * 0.5 * viewport_h;

        if draw_volume {
            volume_flag = 1.0;
        }
    } else {
        // Body rendering: normal bar from open to close
        let y_center_viewport = ((instance.open + instance.close) * 0.5 * sy) + by;
        let half_height_viewport = (instance.close - instance.open) * 0.5 * sy;

        // Use exact bar width in pixels
        let half_bar_width_px = bar_width_px * 0.5;

        // Calculate x position in pixels with pixel alignment
        x_px_in_viewport = x_center_px_aligned + vertex.position.x * bar_width_px;

        // Calculate y in clip space then convert to pixels
        let y_viewport = y_center_viewport + vertex.position.y * 2.0 * half_height_viewport;
        y_px_in_viewport = (1.0 - y_viewport) * 0.5 * viewport_h;
    }

    // Add viewport offset to get window pixel coordinates
    let x_px_window = x_px_in_viewport + viewport_x;
    let y_px_window = y_px_in_viewport + viewport_y;

    // Convert window pixel coordinates to window clip space
    let x_clip = (x_px_window / window_w) * 2.0 - 1.0;
    let y_clip = 1.0 - (y_px_window / window_h) * 2.0;

    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);

    // Color based on flags:
    // bit 0: direction (1=up/green, 0=down/red)
    // bit 1: market hours (2=market-hours/bright, 0=off-hours/dark)
    let direction = instance.flags & 1u;
    let is_market_hours = (instance.flags & 2u) != 0u;

    var base_color: vec3<f32>;
    if (direction == 1u) {
        // Green for up candles
        if (is_market_hours) {
            base_color = view.candle_up_market.rgb;
        } else {
            base_color = view.candle_up_offhours.rgb;
        }
    } else {
        // Red for down candles
        if (is_market_hours) {
            base_color = view.candle_down_market.rgb;
        } else {
            base_color = view.candle_down_offhours.rgb;
        }
    }

    if (is_volume) {
        base_color = view.volume_color.rgb;
        wick_flag = 0.0;
    }

    out.base_color = base_color;
    out.wick_params = vec3<f32>(wick_flag, body_low, body_high);
    out.wick_price = wick_price;
    out.volume_flag = volume_flag;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if (in.volume_flag > 0.5) {
        return vec4<f32>(view.volume_color.rgb, 0.25);
    }

    var color = in.base_color;
    if (in.wick_params.x > 0.5) {
        let body_low = in.wick_params.y;
        let body_high = in.wick_params.z;
        if (in.wick_price < body_low || in.wick_price > body_high) {
            color = view.wick_color.rgb;
        }
    }
    return vec4<f32>(color, 1.0);
}
