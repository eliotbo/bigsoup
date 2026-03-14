use wgpu::util::DeviceExt;

const MAX_INSTANCES: usize = 1000;
const MAX_MARKERS: usize = 64;

/// Check if a timestamp (in nanoseconds) is during regular US stock market trading hours
/// Regular hours: Monday-Friday, 9:30 AM - 4:00 PM ET
fn is_trading_hours(ts_nanos: i64) -> bool {
    use chrono::{DateTime, Datelike, Timelike};
    use chrono_tz::America::New_York;

    let dt_utc = DateTime::from_timestamp_nanos(ts_nanos);
    let dt_et = dt_utc.with_timezone(&New_York);

    let weekday = dt_et.weekday().number_from_monday();
    if weekday > 5 {
        return false;
    }

    let hour = dt_et.hour();
    let minute = dt_et.minute();

    if hour < 9 || hour >= 16 {
        return false;
    }
    if hour == 9 && minute < 30 {
        return false;
    }

    true
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewUniform {
    pub affine: [f32; 4],   // sx, bx, sy, by
    pub viewport: [f32; 4], // x, y, w, h
    pub window: [f32; 4],   // w, h, bar_width_px, volume_enabled_flag
    pub candle_up_market: [f32; 4],    // RGB + padding
    pub candle_up_offhours: [f32; 4],  // RGB + padding
    pub candle_down_market: [f32; 4],  // RGB + padding
    pub candle_down_offhours: [f32; 4],// RGB + padding
    pub wick_color: [f32; 4],          // RGB + padding
    pub volume_color: [f32; 4],        // RGB + padding
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    tick_idx_rel: f32,
    open: f32,
    close: f32,
    high: f32,
    low: f32,
    flags: u32,
    volume_norm: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    kind: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkerInstance {
    tick_idx_rel: f32,
    price_norm: f32,
    radius_px: f32,
    _pad0: f32,
    color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkerVertex {
    position: [f32; 2],
}

pub struct MarkerInstanceData {
    pub tick_idx_rel: f32,
    pub price_norm: f32,
    pub radius_px: f32,
    pub color: [f32; 4],
}

pub struct BarRenderer {
    pipeline: wgpu::RenderPipeline,
    marker_pipeline: wgpu::RenderPipeline,
    pub view_uniform: ViewUniform,
    view_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    volume_vertex_buffer: wgpu::Buffer,
    marker_vertex_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    live_instance_buffer: wgpu::Buffer,
    marker_instance_buffer: wgpu::Buffer,
    instance_count: u32,
    live_instance_count: u32,
    marker_instance_count: u32,
    volume_enabled: bool,
}

impl BarRenderer {
    fn create_instance_buffer(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (std::mem::size_of::<Instance>() * MAX_INSTANCES) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn create_marker_instance_buffer(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (std::mem::size_of::<MarkerInstance>() * MAX_MARKERS) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        viewport_x: f32,
        viewport_y: f32,
        viewport_w: f32,
        viewport_h: f32,
        window_w: f32,
        window_h: f32,
        candle_up_market: [f32; 3],
        candle_up_offhours: [f32; 3],
        candle_down_market: [f32; 3],
        candle_down_offhours: [f32; 3],
        wick_color: [f32; 3],
        volume_color: [f32; 3],
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Bar Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/bar.wgsl").into()),
        });

        let marker_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Live Marker Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/live_marker.wgsl").into()),
        });

        let view_uniform = ViewUniform {
            affine: [1.0, 0.0, 1.0, 0.0],
            viewport: [viewport_x, viewport_y, viewport_w, viewport_h],
            window: [window_w, window_h, 3.0, 0.0],
            candle_up_market: [candle_up_market[0], candle_up_market[1], candle_up_market[2], 0.0],
            candle_up_offhours: [candle_up_offhours[0], candle_up_offhours[1], candle_up_offhours[2], 0.0],
            candle_down_market: [candle_down_market[0], candle_down_market[1], candle_down_market[2], 0.0],
            candle_down_offhours: [candle_down_offhours[0], candle_down_offhours[1], candle_down_offhours[2], 0.0],
            wick_color: [wick_color[0], wick_color[1], wick_color[2], 0.0],
            volume_color: [volume_color[0], volume_color[1], volume_color[2], 0.0],
        };

        let view_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Bar View Buffer"),
            contents: bytemuck::cast_slice(&[view_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bar View Bind Group Layout"),
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bar View Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Bar Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Bar Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![6 => Float32x2, 7 => Float32],
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Instance>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Float32, 1 => Float32, 2 => Float32, 3 => Float32, 4 => Float32, 5 => Uint32, 8 => Float32],
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

        let marker_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Live Marker Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &marker_shader,
                entry_point: Some("vs_main"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<MarkerVertex>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![5 => Float32x2],
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<MarkerInstance>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Float32, 1 => Float32, 2 => Float32, 3 => Float32, 4 => Float32x4],
                    },
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &marker_shader,
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

        let wick_width = 0.5;
        let quad_vertices = [
            Vertex {
                position: [-0.5, -0.5],
                kind: 0.0,
            },
            Vertex {
                position: [0.5, -0.5],
                kind: 0.0,
            },
            Vertex {
                position: [0.5, 0.5],
                kind: 0.0,
            },
            Vertex {
                position: [-0.5, -0.5],
                kind: 0.0,
            },
            Vertex {
                position: [0.5, 0.5],
                kind: 0.0,
            },
            Vertex {
                position: [-0.5, 0.5],
                kind: 0.0,
            },
            Vertex {
                position: [-wick_width, -1.0],
                kind: 1.0,
            },
            Vertex {
                position: [wick_width, -1.0],
                kind: 1.0,
            },
            Vertex {
                position: [wick_width, 1.0],
                kind: 1.0,
            },
            Vertex {
                position: [-wick_width, -1.0],
                kind: 1.0,
            },
            Vertex {
                position: [wick_width, 1.0],
                kind: 1.0,
            },
            Vertex {
                position: [-wick_width, 1.0],
                kind: 1.0,
            },
        ];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Bar Vertex Buffer"),
            contents: bytemuck::cast_slice(&quad_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let volume_vertices = [
            Vertex {
                position: [-0.5, -0.5],
                kind: 2.0,
            },
            Vertex {
                position: [0.5, -0.5],
                kind: 2.0,
            },
            Vertex {
                position: [0.5, 0.5],
                kind: 2.0,
            },
            Vertex {
                position: [-0.5, -0.5],
                kind: 2.0,
            },
            Vertex {
                position: [0.5, 0.5],
                kind: 2.0,
            },
            Vertex {
                position: [-0.5, 0.5],
                kind: 2.0,
            },
        ];

        let volume_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Volume Vertex Buffer"),
            contents: bytemuck::cast_slice(&volume_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let marker_quad = [
            MarkerVertex {
                position: [-1.0, -1.0],
            },
            MarkerVertex {
                position: [1.0, -1.0],
            },
            MarkerVertex {
                position: [1.0, 1.0],
            },
            MarkerVertex {
                position: [-1.0, -1.0],
            },
            MarkerVertex {
                position: [1.0, 1.0],
            },
            MarkerVertex {
                position: [-1.0, 1.0],
            },
        ];

        let marker_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Live Marker Vertex Buffer"),
            contents: bytemuck::cast_slice(&marker_quad),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let instance_buffer = Self::create_instance_buffer(device, "Bar Instance Buffer");
        let live_instance_buffer = Self::create_instance_buffer(device, "Bar Live Instance Buffer");
        let marker_instance_buffer =
            Self::create_marker_instance_buffer(device, "Live Marker Instance Buffer");

        Self {
            pipeline,
            marker_pipeline,
            view_uniform,
            view_buffer,
            bind_group,
            vertex_buffer,
            volume_vertex_buffer,
            marker_vertex_buffer,
            instance_buffer,
            live_instance_buffer,
            marker_instance_buffer,
            instance_count: 0,
            live_instance_count: 0,
            marker_instance_count: 0,
            volume_enabled: false,
        }
    }

    pub fn update_view_uniform(&mut self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.view_buffer,
            0,
            bytemuck::cast_slice(&[self.view_uniform]),
        );
    }

    pub fn set_volume_enabled(&mut self, enabled: bool) {
        self.volume_enabled = enabled;
        self.view_uniform.window[3] = if enabled { 1.0 } else { 0.0 };
    }

    fn write_instances_to_buffer(
        buffer: &wgpu::Buffer,
        count_out: &mut u32,
        candles: &[lod::PlotCandle],
        lod_level: crate::zoom::LodLevel,
        queue: &wgpu::Queue,
        min_price: f32,
        max_price: f32,
        num_bars_in_viewport: u32,
        first_bar_position: f32,
        max_volume: f32,
        volume_enabled: bool,
    ) {
        if candles.is_empty() {
            *count_out = 0;
            return;
        }

        if !min_price.is_finite() || !max_price.is_finite() {
            *count_out = 0;
            return;
        }

        let mut price_span = max_price - min_price;
        if price_span.abs() < 1e-6 {
            price_span = 1e-6;
        }

        let viewport_bars = num_bars_in_viewport.max(1) as f32;
        let should_apply_trading_hours = matches!(
            lod_level,
            crate::zoom::LodLevel::S1
                | crate::zoom::LodLevel::S15
                | crate::zoom::LodLevel::S30
                | crate::zoom::LodLevel::M1
                | crate::zoom::LodLevel::M5
                | crate::zoom::LodLevel::M15
                | crate::zoom::LodLevel::M30
                | crate::zoom::LodLevel::H1
                | crate::zoom::LodLevel::H4
        );

        let mut instances = Vec::with_capacity(candles.len().min(MAX_INSTANCES));
        let volume_scale = if volume_enabled && max_volume > 0.0 {
            1.0 / max_volume
        } else {
            0.0
        };
        for (idx, candle) in candles.iter().enumerate().take(MAX_INSTANCES) {
            let bar_position = first_bar_position + idx as f32;
            let tick_idx_rel = (bar_position / viewport_bars) * 2.0 - 1.0;

            let normalize_price =
                |price: f32| -> f32 { ((price - min_price) / price_span) * 2.0 - 1.0 };

            let open = normalize_price(candle.open);
            let close = normalize_price(candle.close);
            let high = normalize_price(candle.high);
            let low = normalize_price(candle.low);

            let is_up = candle.close >= candle.open;
            let is_market_hours = if should_apply_trading_hours {
                is_trading_hours(candle.ts)
            } else {
                true
            };

            let mut flags = if is_up { 1 } else { 0 };
            if is_market_hours {
                flags |= 2;
            }

            let raw_volume = candle.volume.max(0.0);
            let volume_norm = if volume_scale > 0.0 {
                (raw_volume * volume_scale).clamp(0.0, 1.0)
            } else {
                0.0
            };

            instances.push(Instance {
                tick_idx_rel,
                open,
                close,
                high,
                low,
                flags,
                volume_norm,
            });
        }

        *count_out = instances.len() as u32;
        if *count_out == 0 {
            return;
        }

        queue.write_buffer(buffer, 0, bytemuck::cast_slice(&instances));
    }

    pub fn update_instances(
        &mut self,
        candles: &[lod::PlotCandle],
        lod_level: crate::zoom::LodLevel,
        queue: &wgpu::Queue,
        auto_y_scale: bool,
        fixed_y_min: f32,
        fixed_y_max: f32,
    ) -> (f32, f32) {
        let max_volume = if self.volume_enabled {
            candles
                .iter()
                .map(|c| c.volume.max(0.0))
                .fold(0.0_f32, f32::max)
        } else {
            0.0
        };
        self.update_instances_with_offset(
            candles,
            lod_level,
            queue,
            auto_y_scale,
            fixed_y_min,
            fixed_y_max,
            candles.len() as u32,
            0.0,
            max_volume,
        )
    }

    pub fn update_instances_with_offset(
        &mut self,
        candles: &[lod::PlotCandle],
        lod_level: crate::zoom::LodLevel,
        queue: &wgpu::Queue,
        auto_y_scale: bool,
        fixed_y_min: f32,
        fixed_y_max: f32,
        num_bars_in_viewport: u32,
        first_bar_position: f32,
        max_volume: f32,
    ) -> (f32, f32) {
        if candles.is_empty() {
            self.instance_count = 0;
            return (fixed_y_min, fixed_y_max);
        }

        let (min_price, max_price) = if auto_y_scale {
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            for candle in candles {
                min = min.min(candle.low);
                max = max.max(candle.high);
            }

            let price_range = (max - min).max(1e-6);
            let padding = price_range * 0.05;
            (min - padding, max + padding)
        } else {
            (fixed_y_min, fixed_y_max)
        };

        Self::write_instances_to_buffer(
            &self.instance_buffer,
            &mut self.instance_count,
            candles,
            lod_level,
            queue,
            min_price,
            max_price,
            num_bars_in_viewport,
            first_bar_position,
            max_volume,
            self.volume_enabled,
        );

        (min_price, max_price)
    }

    pub fn update_instances_with_range(
        &mut self,
        candles: &[lod::PlotCandle],
        lod_level: crate::zoom::LodLevel,
        queue: &wgpu::Queue,
        min_price: f32,
        max_price: f32,
        num_bars_in_viewport: u32,
        first_bar_position: f32,
        max_volume: f32,
    ) {
        Self::write_instances_to_buffer(
            &self.instance_buffer,
            &mut self.instance_count,
            candles,
            lod_level,
            queue,
            min_price,
            max_price,
            num_bars_in_viewport,
            first_bar_position,
            max_volume,
            self.volume_enabled,
        );
    }

    pub fn update_live_instances_with_range(
        &mut self,
        candles: &[lod::PlotCandle],
        lod_level: crate::zoom::LodLevel,
        queue: &wgpu::Queue,
        min_price: f32,
        max_price: f32,
        num_bars_in_viewport: u32,
        first_bar_position: f32,
        max_volume: f32,
    ) {
        Self::write_instances_to_buffer(
            &self.live_instance_buffer,
            &mut self.live_instance_count,
            candles,
            lod_level,
            queue,
            min_price,
            max_price,
            num_bars_in_viewport,
            first_bar_position,
            max_volume,
            self.volume_enabled,
        );
    }

    pub fn clear_live_instances(&mut self) {
        self.live_instance_count = 0;
    }

    pub fn update_marker_instance(
        &mut self,
        marker: Option<MarkerInstanceData>,
        queue: &wgpu::Queue,
    ) {
        if let Some(marker) = marker {
            let instance = MarkerInstance {
                tick_idx_rel: marker.tick_idx_rel,
                price_norm: marker.price_norm,
                radius_px: marker.radius_px,
                _pad0: 0.0,
                color: marker.color,
            };

            queue.write_buffer(
                &self.marker_instance_buffer,
                0,
                bytemuck::cast_slice(&[instance]),
            );

            self.marker_instance_count = 1;
        } else {
            self.marker_instance_count = 0;
        }
    }

    pub fn draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.draw(0..12, 0..self.instance_count);

        if self.live_instance_count > 0 {
            render_pass.set_vertex_buffer(1, self.live_instance_buffer.slice(..));
            render_pass.draw(0..12, 0..self.live_instance_count);
        }

        if self.volume_enabled {
            if self.instance_count > 0 {
                render_pass.set_vertex_buffer(0, self.volume_vertex_buffer.slice(..));
                render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                render_pass.draw(0..6, 0..self.instance_count);
            }

            if self.live_instance_count > 0 {
                render_pass.set_vertex_buffer(0, self.volume_vertex_buffer.slice(..));
                render_pass.set_vertex_buffer(1, self.live_instance_buffer.slice(..));
                render_pass.draw(0..6, 0..self.live_instance_count);
            }
        }

        if self.marker_instance_count > 0 {
            render_pass.set_pipeline(&self.marker_pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.marker_vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, self.marker_instance_buffer.slice(..));
            render_pass.draw(0..6, 0..self.marker_instance_count);
        }
    }
}
