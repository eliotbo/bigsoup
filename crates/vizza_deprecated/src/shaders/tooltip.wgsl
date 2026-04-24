// Tooltip background shader - draws a semi-transparent rounded rectangle

struct TooltipUniform {
    // rect: x, y, width, height in pixels
    rect: vec4<f32>,
    // window: width, height, pad, pad
    window: vec4<f32>,
    // Background color with alpha
    color: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> tooltip: TooltipUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // in.position is 0-1, scale to tooltip size
    let local_x = in.position.x * tooltip.rect.z;
    let local_y = in.position.y * tooltip.rect.w;

    // Convert to window pixel coordinates
    let x_px = local_x + tooltip.rect.x;
    let y_px = local_y + tooltip.rect.y;

    // Convert to clip space
    let x_clip = (x_px / tooltip.window.x) * 2.0 - 1.0;
    let y_clip = 1.0 - (y_px / tooltip.window.y) * 2.0;

    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);
    out.local_pos = vec2<f32>(local_x, local_y);

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let corner_radius = 6.0;
    let w = tooltip.rect.z;
    let h = tooltip.rect.w;

    // Calculate distance to nearest edge for rounded corners
    let x = in.local_pos.x;
    let y = in.local_pos.y;

    // Check if we're in a corner region
    var dist: f32 = 0.0;

    if x < corner_radius && y < corner_radius {
        // Top-left corner
        dist = distance(vec2<f32>(x, y), vec2<f32>(corner_radius, corner_radius)) - corner_radius;
    } else if x > w - corner_radius && y < corner_radius {
        // Top-right corner
        dist = distance(vec2<f32>(x, y), vec2<f32>(w - corner_radius, corner_radius)) - corner_radius;
    } else if x < corner_radius && y > h - corner_radius {
        // Bottom-left corner
        dist = distance(vec2<f32>(x, y), vec2<f32>(corner_radius, h - corner_radius)) - corner_radius;
    } else if x > w - corner_radius && y > h - corner_radius {
        // Bottom-right corner
        dist = distance(vec2<f32>(x, y), vec2<f32>(w - corner_radius, h - corner_radius)) - corner_radius;
    }

    // Discard pixels outside the rounded rectangle
    if dist > 0.0 {
        discard;
    }

    // Apply subtle edge softening
    var alpha = tooltip.color.a;
    if dist > -1.0 {
        alpha = alpha * (1.0 + dist);
    }

    return vec4<f32>(tooltip.color.rgb, alpha);
}
