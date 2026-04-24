use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewportBgUniform {
    pub viewport_x: f32,
    pub viewport_y: f32,
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub window_w: f32,
    pub window_h: f32,
    pub bg_color_r: f32,
    pub bg_color_g: f32,
    pub bg_color_b: f32,
    _pad0: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BorderUniform {
    pub viewport_x: f32,
    pub viewport_y: f32,
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub window_w: f32,
    pub window_h: f32,
    pub border_width: f32,
    _padding: f32,
    pub border_color: [f32; 3],
    _padding2: f32,
}

pub struct ViewportRenderer {
    bg_pipeline: wgpu::RenderPipeline,
    pub bg_uniform: ViewportBgUniform,
    bg_uniform_buffer: wgpu::Buffer,
    bg_bind_group: wgpu::BindGroup,

    border_pipeline: wgpu::RenderPipeline,
    pub border_uniform: BorderUniform,
    border_uniform_buffer: wgpu::Buffer,
    border_bind_group: wgpu::BindGroup,
}

impl ViewportRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        viewport_x: f32,
        viewport_y: f32,
        viewport_w: f32,
        viewport_h: f32,
        window_w: f32,
        window_h: f32,
        bg_color: [f32; 3],
    ) -> Self {
        // Create viewport background pipeline
        let bg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Viewport Background Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/viewport_bg.wgsl").into()),
        });

        let bg_uniform = ViewportBgUniform {
            viewport_x,
            viewport_y,
            viewport_w,
            viewport_h,
            window_w,
            window_h,
            bg_color_r: bg_color[0],
            bg_color_g: bg_color[1],
            bg_color_b: bg_color[2],
            _pad0: 0.0,
        };

        let bg_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Viewport Background Uniform Buffer"),
            contents: bytemuck::cast_slice(&[bg_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bg_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Viewport Background Bind Group Layout"),
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
            label: Some("Viewport Background Bind Group"),
            layout: &bg_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: bg_uniform_buffer.as_entire_binding(),
            }],
        });

        let bg_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Viewport Background Pipeline Layout"),
            bind_group_layouts: &[&bg_bind_group_layout],
            push_constant_ranges: &[],
        });

        let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Viewport Background Pipeline"),
            layout: Some(&bg_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &bg_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &bg_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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

        // Create border pipeline
        let border_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Border Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/border.wgsl").into()),
        });

        let border_uniform = BorderUniform {
            viewport_x,
            viewport_y,
            viewport_w,
            viewport_h,
            window_w,
            window_h,
            border_width: 1.0,
            _padding: 0.0,
            border_color: [0.6, 0.6, 0.6], // Default unfocused color
            _padding2: 0.0,
        };

        let border_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Border Uniform Buffer"),
            contents: bytemuck::cast_slice(&[border_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let border_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Border Bind Group Layout"),
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

        let border_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Border Bind Group"),
            layout: &border_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: border_uniform_buffer.as_entire_binding(),
            }],
        });

        let border_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Border Pipeline Layout"),
                bind_group_layouts: &[&border_bind_group_layout],
                push_constant_ranges: &[],
            });

        let border_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Border Pipeline"),
            layout: Some(&border_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &border_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &border_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
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

        Self {
            bg_pipeline,
            bg_uniform,
            bg_uniform_buffer,
            bg_bind_group,
            border_pipeline,
            border_uniform,
            border_uniform_buffer,
            border_bind_group,
        }
    }

    pub fn update_bg_uniform(&mut self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.bg_uniform_buffer,
            0,
            bytemuck::cast_slice(&[self.bg_uniform]),
        );
    }

    pub fn update_border_uniform(&mut self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.border_uniform_buffer,
            0,
            bytemuck::cast_slice(&[self.border_uniform]),
        );
    }

    pub fn draw_background<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.bg_pipeline);
        render_pass.set_bind_group(0, &self.bg_bind_group, &[]);
        render_pass.draw(0..6, 0..1);
    }

    pub fn draw_border<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.border_pipeline);
        render_pass.set_bind_group(0, &self.border_bind_group, &[]);
        render_pass.draw(0..8, 0..1);
    }
}
