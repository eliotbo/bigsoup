//! Headless depth-chart renderer.
//!
//! Renders a sideways histogram of bid/ask price levels to an offscreen
//! texture, then reads the pixels back to CPU memory. No window required.
//! After GPU render, price labels are drawn CPU-side into the pixel buffer.

use anyhow::Result;
use wgpu::util::DeviceExt;

use crate::{config::ColorPalette, depth_snapshot::DepthSnapshot};

/// Per-instance data uploaded to the GPU for each price level.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct DepthBarInstance {
    slot_index: f32,
    quantity: f32,
    side: f32, // 0.0 = bid, 1.0 = ask
    color_r: f32,
    color_g: f32,
    color_b: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct DepthUniform {
    max_qty: f32,
    window_w: f32,
    window_h: f32,
    bar_height_px: f32,
    gap_px: f32,
    margin_left: f32,
    _pad0: f32,
    _pad1: f32,
}

const MAX_LEVELS: usize = 2048;
const BAR_HEIGHT_PX: f32 = 5.0;
const GAP_PX: f32 = 2.0;
const MARGIN_LEFT: f32 = 70.0;

// ── Tiny 4×6 bitmap font for digits, period, dollar sign ──────────────
// Each glyph is 4 columns × 6 rows, stored as [u8; 6] where each byte
// holds 4 bits (MSB = left column).

const GLYPH_W: u32 = 4;
const GLYPH_H: u32 = 6;
const GLYPH_SPACING: u32 = 1; // 1px between characters

fn glyph(ch: char) -> [u8; 6] {
    match ch {
        '0' => [0b0110, 0b1001, 0b1001, 0b1001, 0b1001, 0b0110],
        '1' => [0b0010, 0b0110, 0b0010, 0b0010, 0b0010, 0b0111],
        '2' => [0b0110, 0b1001, 0b0010, 0b0100, 0b1000, 0b1111],
        '3' => [0b0110, 0b1001, 0b0010, 0b0001, 0b1001, 0b0110],
        '4' => [0b1010, 0b1010, 0b1010, 0b1111, 0b0010, 0b0010],
        '5' => [0b1111, 0b1000, 0b1110, 0b0001, 0b1001, 0b0110],
        '6' => [0b0110, 0b1000, 0b1110, 0b1001, 0b1001, 0b0110],
        '7' => [0b1111, 0b0001, 0b0010, 0b0100, 0b0100, 0b0100],
        '8' => [0b0110, 0b1001, 0b0110, 0b1001, 0b1001, 0b0110],
        '9' => [0b0110, 0b1001, 0b1001, 0b0111, 0b0001, 0b0110],
        '.' => [0b0000, 0b0000, 0b0000, 0b0000, 0b0000, 0b0100],
        '$' => [0b0100, 0b1111, 0b1000, 0b0110, 0b0001, 0b1110],
        _ =>   [0b0000, 0b0000, 0b0000, 0b0000, 0b0000, 0b0000],
    }
}

/// Draw a string into an RGBA pixel buffer. `x`, `y` is the top-left corner.
fn draw_text(
    pixels: &mut [u8],
    img_w: u32,
    x: u32,
    y: u32,
    text: &str,
    color: [u8; 4],
) {
    let mut cursor_x = x;
    for ch in text.chars() {
        let g = glyph(ch);
        for row in 0..GLYPH_H {
            let bits = g[row as usize];
            for col in 0..GLYPH_W {
                if bits & (0b1000 >> col) != 0 {
                    let px = cursor_x + col;
                    let py = y + row;
                    if px < img_w {
                        let idx = ((py * img_w + px) * 4) as usize;
                        if idx + 3 < pixels.len() {
                            pixels[idx] = color[0];
                            pixels[idx + 1] = color[1];
                            pixels[idx + 2] = color[2];
                            pixels[idx + 3] = color[3];
                        }
                    }
                }
            }
        }
        cursor_x += GLYPH_W + GLYPH_SPACING;
    }
}

/// Width of a rendered string in pixels.
fn text_width(text: &str) -> u32 {
    let n = text.chars().count() as u32;
    if n == 0 { return 0; }
    n * GLYPH_W + (n - 1) * GLYPH_SPACING
}

pub struct DepthRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    texture_format: wgpu::TextureFormat,
    palette: ColorPalette,
    width: u32,
    height: u32,
}

impl DepthRenderer {
    /// Create a headless depth renderer.
    pub fn new(width: u32, height: u32, palette: ColorPalette) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| anyhow::anyhow!("No suitable GPU adapter for headless depth renderer"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Depth Headless Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        ))?;

        let texture_format = wgpu::TextureFormat::Rgba8UnormSrgb;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Depth Bar Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/depth_bar.wgsl").into()),
        });

        let uniform = DepthUniform {
            max_qty: 1.0,
            window_w: width as f32,
            window_h: height as f32,
            bar_height_px: BAR_HEIGHT_PX,
            gap_px: GAP_PX,
            margin_left: MARGIN_LEFT,
            _pad0: 0.0,
            _pad1: 0.0,
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Depth Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Depth Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Depth Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Depth Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let instance_buffer_size =
            (MAX_LEVELS * std::mem::size_of::<DepthBarInstance>()) as wgpu::BufferAddress;

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Depth Instance Buffer"),
            size: instance_buffer_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Depth Bar Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<DepthBarInstance>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0,  shader_location: 0, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: 4,  shader_location: 1, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: 8,  shader_location: 2, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: 12, shader_location: 3, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: 16, shader_location: 4, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: 20, shader_location: 5, format: wgpu::VertexFormat::Float32 },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: texture_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            uniform_buffer,
            bind_group,
            instance_buffer,
            texture_format,
            palette,
            width,
            height,
        })
    }

    /// Render the given snapshot to RGBA pixel data (width * height * 4 bytes).
    /// Price labels are drawn into the left margin.
    pub fn render_to_pixels(&self, snapshot: &DepthSnapshot) -> Result<Vec<u8>> {
        let (instances, uniform, price_high, tick_size, total_slots) =
            self.prepare_instances(snapshot);
        let instance_count = instances.len() as u32;

        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[uniform]),
        );
        if !instances.is_empty() {
            self.queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&instances),
            );
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Offscreen"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.texture_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let tex_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bytes_per_pixel = 4u32;
        let unpadded_row = self.width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_row = (unpadded_row + align - 1) / align * align;

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Depth Readback"),
            size: (padded_row * self.height) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Depth Offscreen Encoder"),
            });

        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Depth Offscreen Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tex_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.palette.background[0] as f64,
                            g: self.palette.background[1] as f64,
                            b: self.palette.background[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if instance_count > 0 {
                rp.set_pipeline(&self.pipeline);
                rp.set_bind_group(0, &self.bind_group, &[]);
                rp.set_vertex_buffer(0, self.instance_buffer.slice(..));
                rp.draw(0..6, 0..instance_count);
            }
        }

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()??;

        let data = buffer_slice.get_mapped_range();

        let mut pixels = Vec::with_capacity((self.width * self.height * bytes_per_pixel) as usize);
        for y in 0..self.height {
            let start = (y * padded_row) as usize;
            let end = start + unpadded_row as usize;
            pixels.extend_from_slice(&data[start..end]);
        }

        drop(data);
        output_buffer.unmap();

        // Draw Y-axis labels and tick marks into the pixel buffer
        self.draw_y_axis(&mut pixels, price_high, tick_size, total_slots);

        Ok(pixels)
    }

    /// Render the snapshot and save as a PNG file.
    pub fn render_to_png(&self, snapshot: &DepthSnapshot, path: &str) -> Result<()> {
        let pixels = self.render_to_pixels(snapshot)?;

        let file = std::fs::File::create(path)?;
        let w = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(w, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&pixels)?;

        println!("Depth screenshot saved to {}", path);
        Ok(())
    }

    /// Draw price labels and tick marks into the left margin.
    /// Only labels every `label_every` ticks to avoid clutter.
    fn draw_y_axis(
        &self,
        pixels: &mut [u8],
        price_high: f32,
        tick_size: f32,
        total_slots: usize,
    ) {
        let text_color = [200u8, 200, 200, 255];
        let tick_color = [120u8, 120, 120, 255];
        let stride = (BAR_HEIGHT_PX + GAP_PX) as u32;

        // Choose label interval: pick the smallest multiple of tick_size
        // such that labels are at least 10 * GLYPH_H pixels apart.
        let min_label_gap_px = (GLYPH_H + 4) as f32 * 1.5;
        let ticks_per_label = (min_label_gap_px / stride as f32).ceil().max(1.0) as usize;
        // Round up to a "nice" number of ticks (multiples of 5 or 10)
        let ticks_per_label = if ticks_per_label <= 5 {
            5
        } else if ticks_per_label <= 10 {
            10
        } else {
            ((ticks_per_label + 9) / 10) * 10
        };

        for slot in 0..total_slots {
            let price = price_high - slot as f32 * tick_size;
            // Round to cents for clean comparison
            let price_cents = (price * 100.0).round() as i64;

            let bar_y = slot as u32 * stride;
            let bar_centre_y = bar_y + (BAR_HEIGHT_PX as u32) / 2;

            // Only label at multiples of ticks_per_label cents
            let label_interval_cents = (ticks_per_label as f64 * (tick_size * 100.0) as f64).round() as i64;
            if label_interval_cents > 0 && price_cents % label_interval_cents == 0 {
                let label = format!("${:.2}", price_cents as f64 / 100.0);
                let tw = text_width(&label);

                let label_x = if MARGIN_LEFT as u32 > tw + 4 {
                    MARGIN_LEFT as u32 - tw - 4
                } else {
                    1
                };
                let label_y = if bar_centre_y >= GLYPH_H / 2 {
                    bar_centre_y - GLYPH_H / 2
                } else {
                    0
                };

                if label_y + GLYPH_H < self.height {
                    draw_text(pixels, self.width, label_x, label_y, &label, text_color);
                }

                // Tick mark
                let tick_x_start = MARGIN_LEFT as u32 - 3;
                let tick_x_end = MARGIN_LEFT as u32;
                if bar_centre_y < self.height {
                    for x in tick_x_start..tick_x_end {
                        let idx = ((bar_centre_y * self.width + x) * 4) as usize;
                        if idx + 3 < pixels.len() {
                            pixels[idx] = tick_color[0];
                            pixels[idx + 1] = tick_color[1];
                            pixels[idx + 2] = tick_color[2];
                            pixels[idx + 3] = tick_color[3];
                        }
                    }
                }
            }
        }
    }

    /// Build instances with linear price→slot mapping so the spread and any
    /// gaps show up as empty rows.
    ///
    /// Returns (instances, uniform, price_high, tick_size, total_slots).
    fn prepare_instances(
        &self,
        snapshot: &DepthSnapshot,
    ) -> (Vec<DepthBarInstance>, DepthUniform, f32, f32, usize) {
        let bid_color = self.palette.candle_up_market;
        let ask_color = self.palette.candle_down_market;

        if snapshot.bids.is_empty() && snapshot.asks.is_empty() {
            let uniform = DepthUniform {
                max_qty: 1.0,
                window_w: self.width as f32,
                window_h: self.height as f32,
                bar_height_px: BAR_HEIGHT_PX,
                gap_px: GAP_PX,
                margin_left: MARGIN_LEFT,
                _pad0: 0.0,
                _pad1: 0.0,
            };
            return (Vec::new(), uniform, 100.0, 0.01, 0);
        }

        // Determine the full price range across both sides
        let tick_size = 0.01_f32;

        // Highest price = last ask (asks sorted ascending, so last is highest)
        // Lowest price = last bid (bids sorted descending, so last is lowest)
        let price_high = snapshot
            .asks
            .last()
            .map(|&(p, _)| p)
            .unwrap_or(100.0);
        let price_low = snapshot
            .bids
            .last()
            .map(|&(p, _)| p)
            .unwrap_or(99.0);

        // Total number of slots = number of $0.01 ticks from high to low
        let total_slots = ((price_high - price_low) / tick_size).round() as usize + 1;

        // Build a lookup: price (in cents) → (qty, side)
        let mut price_map = std::collections::HashMap::new();
        for &(price, qty) in &snapshot.asks {
            let cents = (price * 100.0).round() as i64;
            price_map.insert(cents, (qty, 1.0_f32, ask_color));
        }
        for &(price, qty) in &snapshot.bids {
            let cents = (price * 100.0).round() as i64;
            price_map.insert(cents, (qty, 0.0_f32, bid_color));
        }

        let mut instances = Vec::with_capacity(total_slots.min(MAX_LEVELS));
        let mut max_log_qty: f32 = 0.0;
        let high_cents = (price_high * 100.0).round() as i64;

        // Minimum log value that gets rendered: log10(1+10) ≈ 1.04
        // We want depth=10 to be at least 3px wide.
        // bar_area = window_w - margin_left. At max_log_qty the bar fills bar_area.
        // So depth=10 → log10(11)/max_log_qty * bar_area >= 3px.
        // We enforce this by clamping max_log_qty so the ratio stays large enough.
        let bar_area = self.width as f32 - MARGIN_LEFT;
        let log_10 = (1.0_f32 + 10.0).log10(); // ≈ 1.04

        for slot in 0..total_slots.min(MAX_LEVELS) {
            let cents = high_cents - slot as i64;
            if let Some(&(qty, side, color)) = price_map.get(&cents) {
                let log_qty = (1.0 + qty).log10();
                max_log_qty = max_log_qty.max(log_qty);
                instances.push(DepthBarInstance {
                    slot_index: slot as f32,
                    quantity: log_qty,
                    side,
                    color_r: color[0],
                    color_g: color[1],
                    color_b: color[2],
                });
            }
        }

        // Ensure depth=10 renders at least 3px: max_log_qty <= log_10 * bar_area / 3
        let max_allowed = log_10 * bar_area / 3.0;
        if max_log_qty <= 0.0 {
            max_log_qty = 1.0;
        } else if max_log_qty > max_allowed {
            max_log_qty = max_allowed;
        }

        let uniform = DepthUniform {
            max_qty: max_log_qty,
            window_w: self.width as f32,
            window_h: self.height as f32,
            bar_height_px: BAR_HEIGHT_PX,
            gap_px: GAP_PX,
            margin_left: MARGIN_LEFT,
            _pad0: 0.0,
            _pad1: 0.0,
        };

        (instances, uniform, price_high, tick_size, total_slots)
    }
}
