//! Headless depth-timeline renderer.
//!
//! Renders a horizontal timeline of LOB depth histograms to an offscreen
//! texture, then reads pixels back to CPU. Price labels and time labels
//! are drawn CPU-side into the pixel buffer.

use anyhow::Result;
use wgpu::util::DeviceExt;

use crate::config::ColorPalette;
use crate::depth_timeline::{
    DepthTimelineInstance, DepthTimelineState, DepthTimelineUniform,
    MAX_INSTANCES, MARGIN_LEFT, MARGIN_BOTTOM, prepare_instances,
};
use crate::price_spacing::select_price_spacing;

// ── Tiny 4×6 bitmap font ───────────────────────────────────────────────

const GLYPH_W: u32 = 4;
const GLYPH_H: u32 = 6;
const GLYPH_SPACING: u32 = 1;

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

fn draw_text(pixels: &mut [u8], img_w: u32, x: u32, y: u32, text: &str, color: [u8; 4]) {
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

fn text_width(text: &str) -> u32 {
    let n = text.chars().count() as u32;
    if n == 0 { return 0; }
    n * GLYPH_W + (n - 1) * GLYPH_SPACING
}

// ── Grid line drawing ───────────────────────────────────────────────────

fn draw_horizontal_line(pixels: &mut [u8], img_w: u32, img_h: u32, y: u32, x_start: u32, x_end: u32, color: [u8; 4]) {
    if y >= img_h { return; }
    let x_end = x_end.min(img_w);
    for x in x_start..x_end {
        let idx = ((y * img_w + x) * 4) as usize;
        if idx + 3 < pixels.len() {
            // Alpha blend for subtle grid lines
            let a = color[3] as f32 / 255.0;
            let inv_a = 1.0 - a;
            pixels[idx] = (color[0] as f32 * a + pixels[idx] as f32 * inv_a) as u8;
            pixels[idx + 1] = (color[1] as f32 * a + pixels[idx + 1] as f32 * inv_a) as u8;
            pixels[idx + 2] = (color[2] as f32 * a + pixels[idx + 2] as f32 * inv_a) as u8;
            pixels[idx + 3] = 255;
        }
    }
}

fn draw_vertical_line(pixels: &mut [u8], img_w: u32, img_h: u32, x: u32, y_start: u32, y_end: u32, color: [u8; 4]) {
    if x >= img_w { return; }
    let y_end = y_end.min(img_h);
    for y in y_start..y_end {
        let idx = ((y * img_w + x) * 4) as usize;
        if idx + 3 < pixels.len() {
            let a = color[3] as f32 / 255.0;
            let inv_a = 1.0 - a;
            pixels[idx] = (color[0] as f32 * a + pixels[idx] as f32 * inv_a) as u8;
            pixels[idx + 1] = (color[1] as f32 * a + pixels[idx + 1] as f32 * inv_a) as u8;
            pixels[idx + 2] = (color[2] as f32 * a + pixels[idx + 2] as f32 * inv_a) as u8;
            pixels[idx + 3] = 255;
        }
    }
}

// ── Renderer ────────────────────────────────────────────────────────────

pub struct DepthTimelineRenderer {
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

impl DepthTimelineRenderer {
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
        .ok_or_else(|| anyhow::anyhow!("No suitable GPU adapter"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("DepthTimeline Headless Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        ))?;

        let texture_format = wgpu::TextureFormat::Rgba8UnormSrgb;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Depth Timeline Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/depth_timeline.wgsl").into()),
        });

        let uniform = DepthTimelineUniform {
            price_min: 0.0,
            price_max: 100.0,
            col_start: 0.0,
            col_count: 1.0,
            max_log_qty: 1.0,
            window_w: width as f32,
            window_h: height as f32,
            column_width_px: 8.0,
            margin_left: MARGIN_LEFT,
            margin_bottom: MARGIN_BOTTOM,
            _pad0: 0.0,
            _pad1: 0.0,
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("DepthTimeline Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("DepthTimeline Bind Group Layout"),
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
            label: Some("DepthTimeline Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("DepthTimeline Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let instance_buffer_size =
            (MAX_INSTANCES * std::mem::size_of::<DepthTimelineInstance>()) as wgpu::BufferAddress;

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DepthTimeline Instance Buffer"),
            size: instance_buffer_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("DepthTimeline Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<DepthTimelineInstance>() as wgpu::BufferAddress,
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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

    /// Build GPU instances from the visible portion of the state.
    fn prepare_instances(
        &self,
        state: &DepthTimelineState,
    ) -> (Vec<DepthTimelineInstance>, DepthTimelineUniform) {
        prepare_instances(state, &self.palette, self.width as f32, self.height as f32)
    }

    /// Render the state to RGBA pixels.
    pub fn render_to_pixels(&self, state: &DepthTimelineState) -> Result<Vec<u8>> {
        let (instances, uniform) = self.prepare_instances(state);
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
            label: Some("DepthTimeline Offscreen"),
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
            label: Some("DepthTimeline Readback"),
            size: (padded_row * self.height) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("DepthTimeline Encoder"),
        });

        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("DepthTimeline Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tex_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.palette.viewport_bg[0] as f64,
                            g: self.palette.viewport_bg[1] as f64,
                            b: self.palette.viewport_bg[2] as f64,
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

        // Draw margin backgrounds (outer area with slightly different color)
        self.draw_margins(&mut pixels);
        // Draw grid lines and labels
        self.draw_grid_and_labels(&mut pixels, state);

        Ok(pixels)
    }

    /// Render and save to PNG.
    pub fn render_to_png(&self, state: &DepthTimelineState, path: &str) -> Result<()> {
        let pixels = self.render_to_pixels(state)?;

        let file = std::fs::File::create(path)?;
        let w = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(w, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&pixels)?;

        println!("Depth timeline screenshot saved to {}", path);
        Ok(())
    }

    /// Draw the margin areas (left margin + bottom margin) with the outer background color,
    /// overwriting the GPU-rendered chart area color in those regions.
    fn draw_margins(&self, pixels: &mut [u8]) {
        let bg = self.palette.background;
        // The GPU clears with viewport_bg; we paint margins with the outer bg color.
        // sRGB texture means the values are already gamma-encoded by the GPU.
        let r = (bg[0].powf(1.0 / 2.2) * 255.0) as u8;
        let g = (bg[1].powf(1.0 / 2.2) * 255.0) as u8;
        let b = (bg[2].powf(1.0 / 2.2) * 255.0) as u8;

        let margin_left = MARGIN_LEFT as u32;
        let margin_bottom = MARGIN_BOTTOM as u32;
        let chart_bottom = self.height.saturating_sub(margin_bottom);

        // Left margin (full height)
        for y in 0..self.height {
            for x in 0..margin_left {
                let idx = ((y * self.width + x) * 4) as usize;
                if idx + 3 < pixels.len() {
                    pixels[idx] = r;
                    pixels[idx + 1] = g;
                    pixels[idx + 2] = b;
                    pixels[idx + 3] = 255;
                }
            }
        }

        // Bottom margin (full width)
        for y in chart_bottom..self.height {
            for x in 0..self.width {
                let idx = ((y * self.width + x) * 4) as usize;
                if idx + 3 < pixels.len() {
                    pixels[idx] = r;
                    pixels[idx + 1] = g;
                    pixels[idx + 2] = b;
                    pixels[idx + 3] = 255;
                }
            }
        }
    }

    fn draw_grid_and_labels(&self, pixels: &mut [u8], state: &DepthTimelineState) {
        let text_color = [200u8, 200, 200, 255];
        let grid_color = [255u8, 255, 255, 30]; // subtle white

        let chart_height = self.height as f32 - MARGIN_BOTTOM;
        let price_range = state.price_max - state.price_min;

        if price_range <= 0.0 || chart_height <= 0.0 {
            return;
        }

        // ── Y-axis: price labels and horizontal grid lines ──────────────
        let spacing = select_price_spacing(price_range, None, 4, 12);
        let ticks = spacing.generate_ticks(state.price_min, state.price_max);

        for &price in &ticks {
            let price_t = (price - state.price_min) / price_range;
            let y_px = (chart_height * (1.0 - price_t)) as u32;

            if y_px >= self.height {
                continue;
            }

            // Horizontal grid line
            draw_horizontal_line(
                pixels,
                self.width,
                self.height,
                y_px,
                MARGIN_LEFT as u32,
                self.width,
                grid_color,
            );

            // Price label
            let label = format!("${}", spacing.format_price(price));
            let tw = text_width(&label);
            let label_x = if MARGIN_LEFT as u32 > tw + 4 {
                MARGIN_LEFT as u32 - tw - 4
            } else {
                1
            };
            let label_y = if y_px >= GLYPH_H / 2 {
                y_px - GLYPH_H / 2
            } else {
                0
            };
            if label_y + GLYPH_H < self.height {
                draw_text(pixels, self.width, label_x, label_y, &label, text_color);
            }
        }

        // ── X-axis: time/tick labels and vertical grid lines ────────────
        let snapshots = state.visible_snapshots();
        let col_start = state.visible_left();
        let num_cols = snapshots.len();

        if num_cols == 0 {
            return;
        }

        // Choose label interval so labels don't overlap
        let label_chars = 6;
        let min_label_gap_px = (label_chars as f32 * (GLYPH_W + GLYPH_SPACING) as f32) + 8.0;
        let cols_per_label_raw = (min_label_gap_px / state.column_width_px).ceil().max(1.0) as usize;
        // Only round up to nice multiples when columns are narrow
        let cols_per_label = if cols_per_label_raw <= 1 {
            1
        } else if cols_per_label_raw <= 5 {
            5
        } else if cols_per_label_raw <= 10 {
            10
        } else {
            ((cols_per_label_raw + 9) / 10) * 10
        };

        let y_label = self.height - MARGIN_BOTTOM as u32 + 4;

        for (i, snap) in snapshots.iter().enumerate() {
            if cols_per_label > 1 {
                let abs_col = col_start + i;
                if abs_col % cols_per_label != 0 {
                    continue;
                }
            }

            let x_px = MARGIN_LEFT as u32 + (i as f32 * state.column_width_px) as u32;
            if x_px >= self.width {
                break;
            }

            // Vertical grid line
            draw_vertical_line(
                pixels,
                self.width,
                self.height,
                x_px,
                0,
                self.height - MARGIN_BOTTOM as u32,
                grid_color,
            );

            // Tick label
            let label = format!("{}", snap.tick);
            if y_label + GLYPH_H < self.height {
                draw_text(pixels, self.width, x_px, y_label, &label, text_color);
            }
        }
    }
}
