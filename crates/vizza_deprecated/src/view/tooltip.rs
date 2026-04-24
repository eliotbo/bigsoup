//! Tooltip renderer for displaying OHLCV data on hover

use chrono::{DateTime, Datelike, Timelike};
use chrono_tz::America::New_York;
use glyphon::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, TextArea, TextAtlas,
    TextBounds, TextRenderer, Viewport,
};
use lod::PlotCandle;
use wgpu::util::DeviceExt;

/// Background quad vertex
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct TooltipVertex {
    position: [f32; 2],
}

/// Uniform data for tooltip background
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct TooltipUniform {
    /// Position and size: [x, y, w, h] in pixels
    rect: [f32; 4],
    /// Window dimensions: [w, h, _, _]
    window: [f32; 4],
    /// Background color with alpha
    color: [f32; 4],
}

/// Data to display in the tooltip
#[derive(Debug, Clone)]
pub struct TooltipData {
    pub timestamp_ns: i64,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    pub volume: f32,
}

/// Viewport bounds for constraining tooltip position
#[derive(Debug, Clone, Copy)]
pub struct ViewportBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl From<&PlotCandle> for TooltipData {
    fn from(candle: &PlotCandle) -> Self {
        TooltipData {
            timestamp_ns: candle.ts,
            open: candle.open,
            high: candle.high,
            low: candle.low,
            close: candle.close,
            volume: candle.volume,
        }
    }
}

pub struct TooltipRenderer {
    bg_pipeline: wgpu::RenderPipeline,
    bg_uniform_buffer: wgpu::Buffer,
    bg_bind_group: wgpu::BindGroup,
    bg_vertex_buffer: wgpu::Buffer,

    // Glyphon text rendering
    font_system: FontSystem,
    swash_cache: SwashCache,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,

    // Current tooltip state
    text_buffer: Option<Buffer>,
    tooltip_x: f32,
    tooltip_y: f32,
    tooltip_w: f32,
    tooltip_h: f32,
    visible: bool,
    window_w: f32,
    window_h: f32,
}

impl TooltipRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        window_w: f32,
        window_h: f32,
    ) -> Self {
        // Create shader for tooltip background
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Tooltip Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/tooltip.wgsl").into()),
        });

        let uniform = TooltipUniform {
            rect: [0.0, 0.0, 200.0, 140.0],
            window: [window_w, window_h, 0.0, 0.0],
            color: [0.1, 0.1, 0.12, 0.92],
        };

        let bg_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Tooltip Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Tooltip Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bg_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Tooltip Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: bg_uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Tooltip Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Tooltip Background Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TooltipVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Quad vertices (triangle strip)
        let vertices = [
            TooltipVertex { position: [0.0, 0.0] },
            TooltipVertex { position: [1.0, 0.0] },
            TooltipVertex { position: [0.0, 1.0] },
            TooltipVertex { position: [1.0, 1.0] },
        ];

        let bg_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Tooltip Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Initialize glyphon
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = glyphon::Cache::new(device);
        let mut text_atlas = TextAtlas::new(device, queue, &cache, format);
        let text_renderer = TextRenderer::new(
            &mut text_atlas,
            device,
            wgpu::MultisampleState::default(),
            None,
        );

        Self {
            bg_pipeline,
            bg_uniform_buffer,
            bg_bind_group,
            bg_vertex_buffer,
            font_system,
            swash_cache,
            text_atlas,
            text_renderer,
            text_buffer: None,
            tooltip_x: 0.0,
            tooltip_y: 0.0,
            tooltip_w: 200.0,
            tooltip_h: 140.0,
            visible: false,
            window_w,
            window_h,
        }
    }

    pub fn update_window_size(&mut self, window_w: f32, window_h: f32) {
        self.window_w = window_w;
        self.window_h = window_h;
    }

    /// Update tooltip with data for a specific candle
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        data: Option<&TooltipData>,
        mouse_x: f32,
        mouse_y: f32,
        viewport_bounds: Option<ViewportBounds>,
    ) {
        if let Some(data) = data {
            self.visible = true;

            // Format timestamp in Eastern Time
            let dt_utc = DateTime::from_timestamp_nanos(data.timestamp_ns);
            let dt_et = dt_utc.with_timezone(&New_York);
            let timestamp_str = format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                dt_et.year(),
                dt_et.month(),
                dt_et.day(),
                dt_et.hour(),
                dt_et.minute()
            );

            // Calculate price change
            let change = data.close - data.open;
            let change_pct = if data.open != 0.0 {
                (change / data.open) * 100.0
            } else {
                0.0
            };
            let change_sign = if change >= 0.0 { "+" } else { "" };

            // Format volume with K/M suffix
            let volume_str = if data.volume >= 1_000_000.0 {
                format!("{:.2}M", data.volume / 1_000_000.0)
            } else if data.volume >= 1_000.0 {
                format!("{:.1}K", data.volume / 1_000.0)
            } else {
                format!("{:.0}", data.volume)
            };

            // Build tooltip text (compact, no double spacing)
            let text = format!(
                "{}\nO: {:.4}\nH: {:.4}\nL: {:.4}\nC: {:.4}\nV: {}\n{}{:.4} ({}{:.2}%)",
                timestamp_str,
                data.open,
                data.high,
                data.low,
                data.close,
                volume_str,
                change_sign,
                change,
                change_sign,
                change_pct.abs()
            );

            // Create text buffer with smaller font
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(10.0, 12.0));
            buffer.set_text(
                &mut self.font_system,
                &text,
                Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
            );
            self.text_buffer = Some(buffer);

            // Calculate tooltip size based on text (smaller)
            self.tooltip_w = 125.0;
            self.tooltip_h = 100.0;

            // Get bounds to constrain tooltip within
            let (bounds_x, bounds_y, bounds_w, bounds_h) = if let Some(vp) = viewport_bounds {
                (vp.x, vp.y, vp.width, vp.height)
            } else {
                (0.0, 0.0, self.window_w, self.window_h)
            };

            // Position tooltip near cursor, staying within viewport bounds
            let offset_x = 12.0;
            let offset_y = 12.0;

            // Default position: to the right and below cursor
            let mut x = mouse_x + offset_x;
            let mut y = mouse_y + offset_y;

            // If tooltip would go off right edge of viewport, flip to left side
            if x + self.tooltip_w > bounds_x + bounds_w {
                x = mouse_x - self.tooltip_w - offset_x;
            }

            // If tooltip would go off bottom edge of viewport, flip to top
            if y + self.tooltip_h > bounds_y + bounds_h {
                y = mouse_y - self.tooltip_h - offset_y;
            }

            // Ensure tooltip stays within viewport bounds
            x = x.clamp(bounds_x + 3.0, bounds_x + bounds_w - self.tooltip_w - 3.0);
            y = y.clamp(bounds_y + 3.0, bounds_y + bounds_h - self.tooltip_h - 3.0);

            self.tooltip_x = x;
            self.tooltip_y = y;

            // Update uniform buffer
            let uniform = TooltipUniform {
                rect: [self.tooltip_x, self.tooltip_y, self.tooltip_w, self.tooltip_h],
                window: [self.window_w, self.window_h, 0.0, 0.0],
                color: [0.1, 0.1, 0.12, 0.92],
            };
            queue.write_buffer(&self.bg_uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
        } else {
            self.visible = false;
            self.text_buffer = None;
        }
    }

    /// Draw tooltip background
    pub fn draw_background<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        if !self.visible {
            return;
        }

        render_pass.set_pipeline(&self.bg_pipeline);
        render_pass.set_bind_group(0, &self.bg_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.bg_vertex_buffer.slice(..));
        render_pass.draw(0..4, 0..1);
    }

    /// Draw tooltip text
    pub fn draw_text<'a>(
        &'a mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_pass: &mut wgpu::RenderPass<'a>,
        glyphon_viewport: &Viewport,
        change_positive: bool,
    ) {
        if !self.visible {
            return;
        }

        let Some(ref buffer) = self.text_buffer else {
            return;
        };

        // Choose color based on price change
        let text_color = if change_positive {
            Color::rgb(100, 220, 120) // Green for positive
        } else {
            Color::rgb(220, 80, 80) // Red for negative
        };

        let text_area = TextArea {
            buffer,
            left: self.tooltip_x + 8.0,
            top: self.tooltip_y + 6.0,
            scale: 1.0,
            bounds: TextBounds {
                left: 0,
                top: 0,
                right: i32::MAX,
                bottom: i32::MAX,
            },
            default_color: text_color,
            custom_glyphs: &[],
        };

        self.text_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.text_atlas,
                glyphon_viewport,
                [text_area],
                &mut self.swash_cache,
            )
            .ok();

        self.text_renderer
            .render(&self.text_atlas, glyphon_viewport, render_pass)
            .ok();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }
}
