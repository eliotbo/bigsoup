use super::bars::ViewUniform;
use wgpu::util::DeviceExt;

const MAX_LINE_VERTICES: usize = 8192;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub position: [f32; 2], // x: bar-space [-1,1], y: normalized price [-1,1]
    pub color: [f32; 4],
}

pub struct LineOverlayRenderer {
    pipeline: wgpu::RenderPipeline,
    view_uniform: ViewUniform,
    view_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
}

impl LineOverlayRenderer {
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
            label: Some("Line Overlay Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/line_overlay.wgsl").into(),
            ),
        });

        let view_uniform = ViewUniform {
            affine: [1.0, 0.0, 1.0, 0.0],
            viewport: [viewport_x, viewport_y, viewport_w, viewport_h],
            window: [window_w, window_h, 0.0, 0.0],
            candle_up_market: [0.0; 4],
            candle_up_offhours: [0.0; 4],
            candle_down_market: [0.0; 4],
            candle_down_offhours: [0.0; 4],
            wick_color: [0.0; 4],
            volume_color: [0.0; 4],
        };

        let view_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Line Overlay View Buffer"),
            contents: bytemuck::cast_slice(&[view_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Line Overlay Bind Group Layout"),
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
            label: Some("Line Overlay Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Line Overlay Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Line Overlay Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x4
                    ],
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

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Line Overlay Vertex Buffer"),
            size: (std::mem::size_of::<LineVertex>() * MAX_LINE_VERTICES) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            view_uniform,
            view_buffer,
            bind_group,
            vertex_buffer,
            vertex_count: 0,
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

    pub fn set_vertices(&mut self, vertices: &[LineVertex], queue: &wgpu::Queue) {
        let capped = vertices.len().min(MAX_LINE_VERTICES);
        self.vertex_count = capped as u32;
        if capped > 0 {
            queue.write_buffer(
                &self.vertex_buffer,
                0,
                bytemuck::cast_slice(&vertices[..capped]),
            );
        }
    }

    pub fn clear(&mut self) {
        self.vertex_count = 0;
    }

    pub fn draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        if self.vertex_count == 0 {
            return;
        }

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..1);
    }
}
