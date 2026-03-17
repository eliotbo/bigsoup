use std::sync::Arc;

use anyhow::Result;
use wgpu::util::DeviceExt;
use winit::{event_loop::ActiveEventLoop, window::Window};

use crate::{config::ColorPalette, depth_snapshot::DepthSnapshot};

/// Per-instance data uploaded to the GPU for each price level.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct DepthBarInstance {
    slot_index: f32,
    quantity: f32,
    side: f32,     // 0.0 = bid, 1.0 = ask
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

pub struct DepthWindow {
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

    palette: ColorPalette,
    needs_redraw: bool,
}

impl DepthWindow {
    pub fn new(event_loop: &ActiveEventLoop, palette: ColorPalette) -> Result<Self> {
        let window_attributes = Window::default_attributes()
            .with_title("Depth – Order Book")
            .with_inner_size(winit::dpi::LogicalSize::new(400u32, 800u32));

        let window = Arc::new(event_loop.create_window(window_attributes)?);
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone())?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| anyhow::anyhow!("No suitable GPU adapter for depth window"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Depth Window Device"),
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Depth Bar Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/depth_bar.wgsl").into(),
            ),
        });

        let uniform = DepthUniform {
            max_qty: 1.0,
            window_w: size.width as f32,
            window_h: size.height as f32,
            bar_height_px: BAR_HEIGHT_PX,
            gap_px: GAP_PX,
            margin_left: 70.0,
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
            palette,
            needs_redraw: true,
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

    /// Upload a new depth snapshot. Call this periodically (e.g. every N ticks).
    /// Uses linear price→slot mapping so the spread shows as empty space.
    pub fn update_snapshot(&mut self, snapshot: &DepthSnapshot) {
        if snapshot.bids.is_empty() && snapshot.asks.is_empty() {
            self.instance_count = 0;
            self.needs_redraw = true;
            return;
        }

        let bid_color = self.palette.candle_up_market;
        let ask_color = self.palette.candle_down_market;
        let tick_size = 0.01_f32;

        let price_high = snapshot.asks.last().map(|&(p, _)| p).unwrap_or(100.0);
        let price_low = snapshot.bids.last().map(|&(p, _)| p).unwrap_or(99.0);
        let total_slots = ((price_high - price_low) / tick_size).round() as usize + 1;

        let mut price_map = std::collections::HashMap::new();
        for &(price, qty) in &snapshot.asks {
            let cents = (price * 100.0).round() as i64;
            price_map.insert(cents, (qty, 1.0_f32, ask_color));
        }
        for &(price, qty) in &snapshot.bids {
            let cents = (price * 100.0).round() as i64;
            price_map.insert(cents, (qty, 0.0_f32, bid_color));
        }

        let high_cents = (price_high * 100.0).round() as i64;
        let mut instances = Vec::with_capacity(total_slots.min(MAX_LEVELS));
        let mut max_log_qty: f32 = 0.0;
        let bar_area = self.surface_config.width as f32 - 70.0;
        let log_10 = (1.0_f32 + 10.0).log10();

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

        let max_allowed = log_10 * bar_area / 3.0;
        if max_log_qty <= 0.0 {
            max_log_qty = 1.0;
        } else if max_log_qty > max_allowed {
            max_log_qty = max_allowed;
        }

        let uniform = DepthUniform {
            max_qty: max_log_qty,
            window_w: self.surface_config.width as f32,
            window_h: self.surface_config.height as f32,
            bar_height_px: BAR_HEIGHT_PX,
            gap_px: GAP_PX,
            margin_left: 70.0,
            _pad0: 0.0,
            _pad1: 0.0,
        };

        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
        self.queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));

        self.instance_count = instances.len() as u32;
        self.needs_redraw = true;
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
                label: Some("Depth Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Depth Render Pass"),
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

            if self.instance_count > 0 {
                render_pass.set_pipeline(&self.pipeline);
                render_pass.set_bind_group(0, &self.bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
                render_pass.draw(0..6, 0..self.instance_count);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        self.needs_redraw = false;
        Ok(())
    }
}
