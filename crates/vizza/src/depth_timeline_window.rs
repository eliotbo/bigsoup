//! Interactive depth-timeline window.
//!
//! Opens a native winit window rendering the LOB depth timeline with the same
//! GPU shader used by `DepthTimelineRenderer`, but drawing to a live surface
//! instead of an offscreen texture. Labels use Glyphon for GPU text rendering.

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
    DepthTimelineInstance, DepthTimelineState, DepthTimelineUniform, MARGIN_BOTTOM, MARGIN_LEFT,
    MAX_INSTANCES, prepare_instances,
};
use crate::price_spacing::select_price_spacing;

pub struct DepthTimelineWindow {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,

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
}

impl DepthTimelineWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        palette: ColorPalette,
        tick_range: (u64, u64),
    ) -> Result<Self> {
        let title = format!("LOB Timeline \u{2014} ticks {}\u{2013}{}", tick_range.0, tick_range.1);
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

        // Compile the depth timeline shader
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
            margin_left: MARGIN_LEFT,
            margin_bottom: MARGIN_BOTTOM,
            _pad0: 0.0,
            _pad1: 0.0,
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

        // Initialize Glyphon
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
        let (instances, uniform) = prepare_instances(state, &self.palette, w, h);

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

        // Rebuild text labels
        self.rebuild_labels(state);

        self.needs_redraw = true;
    }

    fn rebuild_labels(&mut self, state: &DepthTimelineState) {
        self.label_buffers.clear();
        self.label_positions.clear();

        let w = self.surface_config.width as f32;
        let h = self.surface_config.height as f32;
        let chart_height = h - MARGIN_BOTTOM;
        let price_range = state.price_max - state.price_min;

        if price_range <= 0.0 || chart_height <= 0.0 {
            return;
        }

        // ── Y-axis: price labels ──────────────────────────────────────
        let spacing = select_price_spacing(price_range, None, 4, 12);
        let ticks = spacing.generate_ticks(state.price_min, state.price_max);

        for &price in &ticks {
            let price_t = (price - state.price_min) / price_range;
            let y_px = chart_height * (1.0 - price_t);
            if y_px < 0.0 || y_px >= h {
                continue;
            }

            let label = format!("${}", spacing.format_price(price));
            let mut buffer =
                TextBuffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
            buffer.set_text(
                &mut self.font_system,
                &label,
                Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
            );

            // Right-align in left margin
            let label_x = 2.0_f32;
            let label_y = y_px - 9.0; // center vertically on grid line

            self.label_buffers.push(buffer);
            self.label_positions.push((label_x, label_y));
        }

        // ── X-axis: tick labels ───────────────────────────────────────
        let snapshots = state.visible_snapshots();
        let col_start = state.visible_left();
        let num_cols = snapshots.len();
        if num_cols == 0 {
            return;
        }

        // Choose label interval so labels don't overlap
        let min_label_gap_px = 50.0;
        let cols_per_label_raw = (min_label_gap_px / state.column_width_px).ceil().max(1.0) as usize;
        let cols_per_label = if cols_per_label_raw <= 1 {
            1
        } else if cols_per_label_raw <= 5 {
            5
        } else if cols_per_label_raw <= 10 {
            10
        } else {
            ((cols_per_label_raw + 9) / 10) * 10
        };

        let y_label = h - MARGIN_BOTTOM + 4.0;

        for (i, snap) in snapshots.iter().enumerate() {
            if cols_per_label > 1 {
                let abs_col = col_start + i;
                if abs_col % cols_per_label != 0 {
                    continue;
                }
            }

            let x_px = MARGIN_LEFT + i as f32 * state.column_width_px;
            if x_px >= w {
                break;
            }

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
            self.label_positions.push((x_px, y_label));
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

        // Pass 1: draw depth bars
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("DepthTimeline Window Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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

            let text_color = Color::rgba(200, 200, 200, 255);
            let mut text_areas = Vec::new();
            for (i, (x, y)) in self.label_positions.iter().enumerate() {
                text_areas.push(TextArea {
                    buffer: &self.label_buffers[i],
                    left: *x,
                    top: *y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: i32::MAX,
                        bottom: i32::MAX,
                    },
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
    ) {
        if button == winit::event::MouseButton::Left {
            match press {
                winit::event::ElementState::Pressed => {
                    self.dragging = true;
                    self.drag_start_x = cursor_x;
                }
                winit::event::ElementState::Released => {
                    self.dragging = false;
                }
            }
        }
    }

    /// Handle cursor movement. Returns the number of columns to pan.
    pub fn on_cursor_moved(
        &mut self,
        cursor_x: f64,
        state: &mut DepthTimelineState,
    ) {
        if !self.dragging {
            return;
        }
        let delta_px = self.drag_start_x - cursor_x;
        let delta_cols = (delta_px / state.column_width_px as f64) as i32;
        if delta_cols != 0 {
            state.pan_x(delta_cols);
            self.drag_start_x = cursor_x;
        }
    }

    /// Handle scroll wheel for zooming.
    pub fn on_mouse_wheel(
        &mut self,
        delta_y: f32,
        state: &mut DepthTimelineState,
    ) {
        let factor = 1.1_f32.powf(delta_y);
        state.column_width_px = (state.column_width_px * factor).clamp(2.0, 200.0);
        let chart_w = self.surface_config.width as f32 - MARGIN_LEFT;
        state.visible_count = (chart_w / state.column_width_px).ceil() as usize;
        if state.auto_y_scale {
            state.auto_scale_y();
        }
    }

    pub fn on_key(
        &self,
        key: winit::keyboard::KeyCode,
        state: &mut DepthTimelineState,
    ) {
        match key {
            winit::keyboard::KeyCode::Home => {
                state.visible_right = state.visible_count.min(state.timeline.snapshots.len());
            }
            winit::keyboard::KeyCode::End => {
                state.visible_right = state.timeline.snapshots.len();
            }
            _ => {}
        }
    }
}
