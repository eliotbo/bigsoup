//! Interactive depth-timeline window.
//!
//! Opens a native winit window rendering the LOB depth timeline with the same
//! GPU shader used by `DepthTimelineRenderer`, but drawing to a live surface
//! instead of an offscreen texture. Labels use Glyphon for GPU text rendering.
//! The chart is inset from the window edges (like vizza candlestick charts)
//! with a border contour around the viewport.

use std::sync::Arc;

use anyhow::Result;
use glyphon::{
    Attrs, Buffer as TextBuffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use wgpu::util::DeviceExt;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::config::ColorPalette;
use crate::depth_timeline::{
    ChartMargins, DepthTimelineInstance, DepthTimelineState, DepthTimelineUniform, MAX_INSTANCES,
    prepare_instances,
};
use crate::price_spacing::select_price_spacing;

// ── Viewport inset ────────────────────────────────────────────────────

/// Margin between window edge and chart viewport (matches vizza candlestick charts).
const VIEWPORT_MARGIN: f32 = 50.0;

// ── Grid line vertex ──────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GridLineVertex {
    position: [f32; 2], // pixel coords
    color: [f32; 4],    // RGBA
}

const GRID_LINE_SHADER: &str = r#"
struct Uniform {
    window_w: f32,
    window_h: f32,
}

@group(0) @binding(0) var<uniform> u: Uniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let x_clip = (in.position.x / u.window_w) * 2.0 - 1.0;
    let y_clip = 1.0 - (in.position.y / u.window_h) * 2.0;
    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GridUniform {
    window_w: f32,
    window_h: f32,
}

const MAX_GRID_VERTICES: usize = 500; // ~200 grid lines + 4 border lines

// ── Window ────────────────────────────────────────────────────────────

pub struct DepthTimelineWindow {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,

    // Depth bar pipeline
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,

    // Grid line pipeline (also used for border)
    grid_pipeline: wgpu::RenderPipeline,
    grid_uniform_buffer: wgpu::Buffer,
    grid_bind_group: wgpu::BindGroup,
    grid_vertex_buffer: wgpu::Buffer,
    grid_vertex_count: u32,

    // Viewport background fill pipeline (TriangleList, same shader as grid)
    fill_pipeline: wgpu::RenderPipeline,
    fill_vertex_buffer: wgpu::Buffer,

    // Glyphon text rendering
    font_system: FontSystem,
    swash_cache: SwashCache,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,

    // Label data
    label_buffers: Vec<TextBuffer>,
    label_positions: Vec<(f32, f32)>,

    palette: ColorPalette,
    needs_redraw: bool,

    // Drag state
    dragging: bool,
    drag_start_x: f64,
    drag_start_y: f64,
}

impl DepthTimelineWindow {
    /// Compute the chart viewport rect for a given window size.
    fn viewport_rect(w: f32, h: f32) -> (f32, f32, f32, f32) {
        let x = VIEWPORT_MARGIN;
        let y = VIEWPORT_MARGIN;
        let vw = (w - 2.0 * VIEWPORT_MARGIN).max(1.0);
        let vh = (h - 2.0 * VIEWPORT_MARGIN).max(1.0);
        (x, y, vw, vh)
    }

    /// Compute shader margins from the viewport rect.
    fn chart_margins(w: f32, h: f32) -> ChartMargins {
        let (vx, vy, vw, vh) = Self::viewport_rect(w, h);
        ChartMargins {
            left: vx,
            top: vy,
            right: w - vx - vw,
            bottom: h - vy - vh,
        }
    }

    pub fn new(
        event_loop: &ActiveEventLoop,
        palette: ColorPalette,
        tick_range: (u64, u64),
    ) -> Result<Self> {
        let title = format!(
            "LOB Timeline \u{2014} ticks {}\u{2013}{}",
            tick_range.0, tick_range.1
        );
        let window_attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(1400u32, 800u32));

        let window = Arc::new(event_loop.create_window(window_attributes)?);
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone())?;

        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            }))
            .ok_or_else(|| anyhow::anyhow!("No suitable GPU adapter for timeline window"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("DepthTimeline Window Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        ))?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let margins = Self::chart_margins(size.width as f32, size.height as f32);

        // ── Depth bar pipeline ────────────────────────────────────────
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Depth Timeline Window Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/depth_timeline.wgsl").into(),
            ),
        });

        let uniform = DepthTimelineUniform {
            price_min: 0.0,
            price_max: 100.0,
            col_start: 0.0,
            col_count: 1.0,
            max_log_qty: 1.0,
            window_w: size.width as f32,
            window_h: size.height as f32,
            column_width_px: 8.0,
            margin_left: margins.left,
            margin_bottom: margins.bottom,
            margin_top: margins.top,
            margin_right: margins.right,
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("DepthTimeline Window Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("DepthTimeline Window Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DepthTimeline Window Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("DepthTimeline Window Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let instance_buffer_size =
            (MAX_INSTANCES * std::mem::size_of::<DepthTimelineInstance>()) as wgpu::BufferAddress;

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DepthTimeline Window Instance Buffer"),
            size: instance_buffer_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("DepthTimeline Window Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<DepthTimelineInstance>()
                        as wgpu::BufferAddress,
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
                    format: surface_format,
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

        // ── Grid line pipeline ────────────────────────────────────────
        let grid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Grid Line Shader"),
            source: wgpu::ShaderSource::Wgsl(GRID_LINE_SHADER.into()),
        });

        let grid_uniform = GridUniform {
            window_w: size.width as f32,
            window_h: size.height as f32,
        };

        let grid_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Grid Uniform Buffer"),
            contents: bytemuck::cast_slice(&[grid_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let grid_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Grid Bind Group Layout"),
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

        let grid_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Grid Bind Group"),
            layout: &grid_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: grid_uniform_buffer.as_entire_binding(),
            }],
        });

        let grid_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Grid Pipeline Layout"),
                bind_group_layouts: &[&grid_bind_group_layout],
                push_constant_ranges: &[],
            });

        let grid_vertex_buffer_size =
            (MAX_GRID_VERTICES * std::mem::size_of::<GridLineVertex>()) as wgpu::BufferAddress;

        let grid_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Grid Vertex Buffer"),
            size: grid_vertex_buffer_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let grid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Grid Line Pipeline"),
            layout: Some(&grid_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &grid_shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GridLineVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: 8,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &grid_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
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

        // ── Viewport background fill pipeline (same shader, TriangleList) ──
        let fill_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Viewport Fill Pipeline"),
            layout: Some(&grid_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &grid_shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GridLineVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: 8,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &grid_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
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

        // 6 vertices for a viewport-filling quad
        let fill_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Viewport Fill Vertex Buffer"),
            size: (6 * std::mem::size_of::<GridLineVertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Glyphon ──────────────────────────────────────────────────
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = glyphon::Cache::new(&device);
        let mut text_atlas = TextAtlas::new(&device, &queue, &cache, surface_format);
        let text_renderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );

        Ok(Self {
            window,
            surface,
            device,
            queue,
            surface_config,
            pipeline,
            uniform_buffer,
            bind_group,
            instance_buffer,
            instance_count: 0,
            grid_pipeline,
            grid_uniform_buffer,
            grid_bind_group,
            grid_vertex_buffer,
            grid_vertex_count: 0,
            fill_pipeline,
            fill_vertex_buffer,
            font_system,
            swash_cache,
            text_atlas,
            text_renderer,
            label_buffers: Vec::new(),
            label_positions: Vec::new(),
            palette,
            needs_redraw: true,
            dragging: false,
            drag_start_x: 0.0,
            drag_start_y: 0.0,
        })
    }

    pub fn window_id(&self) -> winit::window::WindowId {
        self.window.id()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.surface_config.width = new_size.width;
            self.surface_config.height = new_size.height;
            self.surface.configure(&self.device, &self.surface_config);
            self.needs_redraw = true;
        }
    }

    /// Upload new data from state. Call after any state change.
    pub fn update(&mut self, state: &DepthTimelineState) {
        let w = self.surface_config.width as f32;
        let h = self.surface_config.height as f32;
        let margins = Self::chart_margins(w, h);
        let (instances, uniform) = prepare_instances(state, &self.palette, w, h, margins);

        self.instance_count = instances.len() as u32;

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

        // Update grid uniform with current window size
        let grid_uniform = GridUniform {
            window_w: w,
            window_h: h,
        };
        self.queue.write_buffer(
            &self.grid_uniform_buffer,
            0,
            bytemuck::cast_slice(&[grid_uniform]),
        );

        // Upload viewport background fill quad
        let (vx, vy, vw, vh) = Self::viewport_rect(w, h);
        let bg = self.palette.viewport_bg;
        let bg_color = [bg[0], bg[1], bg[2], 1.0];
        let fill_verts = [
            GridLineVertex { position: [vx,      vy],      color: bg_color },
            GridLineVertex { position: [vx,      vy + vh], color: bg_color },
            GridLineVertex { position: [vx + vw, vy],      color: bg_color },
            GridLineVertex { position: [vx + vw, vy],      color: bg_color },
            GridLineVertex { position: [vx,      vy + vh], color: bg_color },
            GridLineVertex { position: [vx + vw, vy + vh], color: bg_color },
        ];
        self.queue.write_buffer(
            &self.fill_vertex_buffer,
            0,
            bytemuck::cast_slice(&fill_verts),
        );

        // Rebuild grid lines, border, and text labels
        self.rebuild_grid_and_labels(state);

        self.needs_redraw = true;
    }

    fn rebuild_grid_and_labels(&mut self, state: &DepthTimelineState) {
        self.label_buffers.clear();
        self.label_positions.clear();

        let w = self.surface_config.width as f32;
        let h = self.surface_config.height as f32;
        let (vx, vy, vw, vh) = Self::viewport_rect(w, h);

        let price_range = state.price_max - state.price_min;

        let grid_color = self.palette.grid_line;
        let border_color: [f32; 4] = [0.6, 0.6, 0.6, 1.0]; // same as vizza viewport border

        let mut grid_verts: Vec<GridLineVertex> = Vec::new();

        // ── Border contour (4 lines around viewport) ─────────────────
        let push_line = |verts: &mut Vec<GridLineVertex>,
                         x0: f32, y0: f32, x1: f32, y1: f32,
                         color: [f32; 4]| {
            if verts.len() + 2 <= MAX_GRID_VERTICES {
                verts.push(GridLineVertex { position: [x0, y0], color });
                verts.push(GridLineVertex { position: [x1, y1], color });
            }
        };
        // Top
        push_line(&mut grid_verts, vx, vy, vx + vw, vy, border_color);
        // Right
        push_line(&mut grid_verts, vx + vw, vy, vx + vw, vy + vh, border_color);
        // Bottom
        push_line(&mut grid_verts, vx + vw, vy + vh, vx, vy + vh, border_color);
        // Left
        push_line(&mut grid_verts, vx, vy + vh, vx, vy, border_color);

        if price_range <= 0.0 || vh <= 0.0 {
            self.grid_vertex_count = grid_verts.len() as u32;
            if !grid_verts.is_empty() {
                self.queue.write_buffer(
                    &self.grid_vertex_buffer,
                    0,
                    bytemuck::cast_slice(&grid_verts),
                );
            }
            return;
        }

        // ── Y-axis: price grid lines + labels (right side, inside) ───
        let spacing = select_price_spacing(price_range, None, 4, 12);
        let ticks = spacing.generate_ticks(state.price_min, state.price_max);

        for &price in &ticks {
            let price_t = (price - state.price_min) / price_range;
            let y_px = vy + vh * (1.0 - price_t);
            if y_px < vy || y_px > vy + vh {
                continue;
            }

            // Horizontal grid line across chart area
            push_line(&mut grid_verts, vx, y_px, vx + vw, y_px, grid_color);

            // Price label — right side, inside the chart
            let label = format!("${}", spacing.format_price(price));
            let mut buffer =
                TextBuffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
            buffer.set_text(
                &mut self.font_system,
                &label,
                Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
            );

            // Position at right edge of viewport, offset inward
            let label_x = vx + vw - 65.0;
            let label_y = y_px + 2.0; // slightly below the grid line

            self.label_buffers.push(buffer);
            self.label_positions.push((label_x, label_y));
        }

        // ── X-axis: tick grid lines + labels (inside, near bottom) ───
        let (range_start, range_end) = state.visible_range();
        let scroll_left = state.scroll_left();

        if range_start < range_end {
            let min_label_gap_px = 50.0;
            let cols_per_label_raw =
                (min_label_gap_px / state.column_width_px).ceil().max(1.0) as usize;
            let cols_per_label = if cols_per_label_raw <= 1 {
                1
            } else if cols_per_label_raw <= 5 {
                5
            } else if cols_per_label_raw <= 10 {
                10
            } else {
                ((cols_per_label_raw + 9) / 10) * 10
            };

            // Labels inside the chart, near the bottom
            let y_label = vy + vh - 18.0;

            for abs_col in range_start..range_end {
                if cols_per_label > 1 && abs_col % cols_per_label != 0 {
                    continue;
                }

                // Position using the continuous scroll offset
                let x_px = vx + (abs_col as f32 - scroll_left) * state.column_width_px;
                if x_px > vx + vw {
                    break;
                }
                if x_px < vx {
                    continue;
                }

                // Vertical grid line (already inside viewport)
                push_line(&mut grid_verts, x_px, vy, x_px, vy + vh, grid_color);

                // Tick label
                let snap = &state.timeline.snapshots[abs_col];
                let label = format!("{}", snap.tick);
                let mut buffer =
                    TextBuffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
                buffer.set_text(
                    &mut self.font_system,
                    &label,
                    Attrs::new().family(Family::Monospace),
                    Shaping::Advanced,
                );

                self.label_buffers.push(buffer);
                self.label_positions.push((x_px + 2.0, y_label));
            }
        }

        // Upload grid vertices
        self.grid_vertex_count = grid_verts.len() as u32;
        if !grid_verts.is_empty() {
            self.queue.write_buffer(
                &self.grid_vertex_buffer,
                0,
                bytemuck::cast_slice(&grid_verts),
            );
        }
    }

    pub fn render(&mut self) -> Result<()> {
        if !self.needs_redraw {
            return Ok(());
        }

        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("DepthTimeline Window Encoder"),
            });

        // Pass 1: clear + grid lines + border + depth bars
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("DepthTimeline Window Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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

            // Viewport background fill (behind everything)
            rp.set_pipeline(&self.fill_pipeline);
            rp.set_bind_group(0, &self.grid_bind_group, &[]);
            rp.set_vertex_buffer(0, self.fill_vertex_buffer.slice(..));
            rp.draw(0..6, 0..1);

            // Grid lines + border (behind bars)
            if self.grid_vertex_count > 0 {
                rp.set_pipeline(&self.grid_pipeline);
                rp.set_bind_group(0, &self.grid_bind_group, &[]);
                rp.set_vertex_buffer(0, self.grid_vertex_buffer.slice(..));
                rp.draw(0..self.grid_vertex_count, 0..1);
            }

            // Depth bars on top
            if self.instance_count > 0 {
                rp.set_pipeline(&self.pipeline);
                rp.set_bind_group(0, &self.bind_group, &[]);
                rp.set_vertex_buffer(0, self.instance_buffer.slice(..));
                rp.draw(0..6, 0..self.instance_count);
            }
        }

        // Pass 2: Glyphon text labels
        if !self.label_buffers.is_empty() {
            let mut viewport = Viewport::new(&self.device, &glyphon::Cache::new(&self.device));
            viewport.update(
                &self.queue,
                glyphon::Resolution {
                    width: self.surface_config.width,
                    height: self.surface_config.height,
                },
            );

            let tc = self.palette.text_secondary;
            let text_color = Color::rgba(tc[0], tc[1], tc[2], tc[3]);

            // Clip text to viewport rect
            let w = self.surface_config.width as f32;
            let h = self.surface_config.height as f32;
            let (vx, vy, vw, vh) = Self::viewport_rect(w, h);
            let clip_bounds = TextBounds {
                left: vx as i32,
                top: vy as i32,
                right: (vx + vw) as i32,
                bottom: (vy + vh) as i32,
            };

            let mut text_areas = Vec::new();
            for (i, (x, y)) in self.label_positions.iter().enumerate() {
                text_areas.push(TextArea {
                    buffer: &self.label_buffers[i],
                    left: *x,
                    top: *y,
                    scale: 1.0,
                    bounds: clip_bounds,
                    default_color: text_color,
                    custom_glyphs: &[],
                });
            }

            self.text_renderer
                .prepare(
                    &self.device,
                    &self.queue,
                    &mut self.font_system,
                    &mut self.text_atlas,
                    &viewport,
                    text_areas,
                    &mut self.swash_cache,
                )
                .ok();

            {
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("DepthTimeline Text Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                self.text_renderer
                    .render(&self.text_atlas, &viewport, &mut rp)
                    .ok();
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        self.needs_redraw = false;
        Ok(())
    }

    // ── Input handling ────────────────────────────────────────────────

    pub fn on_mouse_input(
        &mut self,
        button: winit::event::MouseButton,
        press: winit::event::ElementState,
        cursor_x: f64,
        cursor_y: f64,
    ) {
        if button == winit::event::MouseButton::Left {
            match press {
                winit::event::ElementState::Pressed => {
                    self.dragging = true;
                    self.drag_start_x = cursor_x;
                    self.drag_start_y = cursor_y;
                }
                winit::event::ElementState::Released => {
                    self.dragging = false;
                }
            }
        }
    }

    /// Handle cursor movement for smooth panning.
    pub fn on_cursor_moved(
        &mut self,
        cursor_x: f64,
        cursor_y: f64,
        state: &mut DepthTimelineState,
    ) {
        if !self.dragging {
            return;
        }
        // Horizontal pan
        let delta_px_x = (self.drag_start_x - cursor_x) as f32;
        self.drag_start_x = cursor_x;
        let delta_cols = delta_px_x / state.column_width_px;
        state.pan_x(delta_cols);

        // Vertical pan (only when auto Y-scale is off)
        if !state.auto_y_scale {
            let delta_px_y = (cursor_y - self.drag_start_y) as f32;
            self.drag_start_y = cursor_y;
            let w = self.surface_config.width as f32;
            let h = self.surface_config.height as f32;
            let (_, _, _, vh) = Self::viewport_rect(w, h);
            let price_range = state.price_max - state.price_min;
            // Dragging down = cursor_y increases = delta_px_y negative = shift prices down
            let delta_price = delta_px_y * price_range / vh;
            state.price_min += delta_price;
            state.price_max += delta_price;
        } else {
            self.drag_start_y = cursor_y;
        }
    }

    /// Handle scroll wheel for zooming.
    pub fn on_mouse_wheel(&mut self, delta_y: f32, state: &mut DepthTimelineState) {
        let factor = 1.1_f32.powf(delta_y);
        state.column_width_px = (state.column_width_px * factor).clamp(2.0, 200.0);
        let w = self.surface_config.width as f32;
        let h = self.surface_config.height as f32;
        let (_, _, vw, _) = Self::viewport_rect(w, h);
        state.visible_count = (vw / state.column_width_px).ceil() as usize;
        // Re-clamp scroll position for new visible_count
        let max_right = state.timeline.snapshots.len() as f32 + state.visible_count as f32;
        state.scroll_right = state.scroll_right.clamp(0.0, max_right);
        if state.auto_y_scale {
            state.auto_scale_y();
        }
    }

    pub fn on_key(&self, key: winit::keyboard::KeyCode, state: &mut DepthTimelineState) {
        match key {
            winit::keyboard::KeyCode::Home => {
                state.scroll_right = state.visible_count as f32;
            }
            winit::keyboard::KeyCode::End => {
                state.scroll_right = state.timeline.snapshots.len() as f32;
            }
            winit::keyboard::KeyCode::KeyY => {
                state.auto_y_scale = !state.auto_y_scale;
                if state.auto_y_scale {
                    state.auto_scale_y();
                }
            }
            _ => {}
        }
    }
}
