use super::bars::ViewUniform;
use wgpu::util::DeviceExt;

const MAX_QUAD_INSTANCES: usize = 256;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct QuadVertex {
    position: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct QuadInstance {
    x_start: f32,
    x_end: f32,
    y_start: f32,
    y_end: f32,
    color: [f32; 4],
}

#[derive(Copy, Clone, Debug)]
pub struct PriceLevelQuadInstanceData {
    pub x_start: f32,
    pub x_end: f32,
    pub y_start: f32,
    pub y_end: f32,
    pub color: [f32; 4],
}

pub struct PriceLevelQuadRenderer {
    pipeline: wgpu::RenderPipeline,
    view_uniform: ViewUniform,
    view_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
}

impl PriceLevelQuadRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        viewport_x: f32,
        viewport_y: f32,
        viewport_w: f32,
        viewport_h: f32,
        window_w: f32,
        window_h: f32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Price Level Quad Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/price_level_quad.wgsl").into(),
            ),
        });

        let view_uniform = ViewUniform {
            affine: [1.0, 0.0, 1.0, 0.0],
            viewport: [viewport_x, viewport_y, viewport_w, viewport_h],
            window: [window_w, window_h, 0.0, 0.0],
            candle_up_market: [0.0, 0.0, 0.0, 0.0],
            candle_up_offhours: [0.0, 0.0, 0.0, 0.0],
            candle_down_market: [0.0, 0.0, 0.0, 0.0],
            candle_down_offhours: [0.0, 0.0, 0.0, 0.0],
            wick_color: [0.0, 0.0, 0.0, 0.0],
            volume_color: [0.0, 0.0, 0.0, 0.0],
        };

        let view_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Price Level Quad View Buffer"),
            contents: bytemuck::cast_slice(&[view_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Price Level Quad Bind Group Layout"),
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
            label: Some("Price Level Quad Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Price Level Quad Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Price Level Quad Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<QuadVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<QuadInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            1 => Float32,
                            2 => Float32,
                            3 => Float32,
                            4 => Float32,
                            5 => Float32x4
                        ],
                    },
                ],
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

        let quad_vertices = [
            QuadVertex {
                position: [-1.0, -1.0],
            },
            QuadVertex {
                position: [1.0, -1.0],
            },
            QuadVertex {
                position: [1.0, 1.0],
            },
            QuadVertex {
                position: [-1.0, -1.0],
            },
            QuadVertex {
                position: [1.0, 1.0],
            },
            QuadVertex {
                position: [-1.0, 1.0],
            },
        ];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Price Level Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(&quad_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Price Level Quad Instance Buffer"),
            size: (std::mem::size_of::<QuadInstance>() * MAX_QUAD_INSTANCES) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            view_uniform,
            view_buffer,
            bind_group,
            vertex_buffer,
            instance_buffer,
            instance_count: 0,
        }
    }

    pub fn update_view_uniform(&mut self, view: &ViewUniform, queue: &wgpu::Queue) {
        self.view_uniform = *view;
        queue.write_buffer(
            &self.view_buffer,
            0,
            bytemuck::cast_slice(&[self.view_uniform]),
        );
    }

    pub fn set_instances(
        &mut self,
        quads: &[PriceLevelQuadInstanceData],
        queue: &wgpu::Queue,
        min_price: f32,
        max_price: f32,
    ) {
        if quads.is_empty() {
            self.instance_count = 0;
            return;
        }

        // Normalize price values to [-1, 1] range (same as bars)
        let mut price_span = max_price - min_price;
        if price_span.abs() < 1e-6 {
            price_span = 1e-6;
        }

        let normalize_price = |price: f32| -> f32 {
            ((price - min_price) / price_span) * 2.0 - 1.0
        };

        let capped_len = quads.len().min(MAX_QUAD_INSTANCES);
        let mut gpu_instances = Vec::with_capacity(capped_len);
        for instance in quads.iter().take(capped_len) {
            gpu_instances.push(QuadInstance {
                x_start: instance.x_start,
                x_end: instance.x_end,
                y_start: normalize_price(instance.y_start),
                y_end: normalize_price(instance.y_end),
                color: instance.color,
            });
        }

        self.instance_count = gpu_instances.len() as u32;
        queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&gpu_instances),
        );
    }

    pub fn clear_instances(&mut self) {
        self.instance_count = 0;
    }

    pub fn draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.draw(0..6, 0..self.instance_count);
    }
}
