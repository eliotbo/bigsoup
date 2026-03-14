use crate::loader::DividendWithIndex;
use crate::price_spacing::{PriceTickSpacing, select_price_spacing};
use crate::zoom::LodLevel;
use chrono::{DateTime, Datelike, Timelike};
use chrono_tz::America::New_York;
use glyphon::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, TextArea, TextAtlas,
    TextBounds, TextRenderer, Viewport,
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GridUniform {
    viewport: [f32; 4], // x, y, w, h
    window: [f32; 4],   // w, h, _pad0, _pad1
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GridVertex {
    position: [f32; 2], // Viewport-relative coordinates (0.0 to 1.0)
    color: [f32; 4],    // RGBA color with alpha
}

pub struct GridRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,

    // Glyphon text rendering
    font_system: FontSystem,
    swash_cache: SwashCache,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,

    // Label data (time labels at bottom)
    label_buffers: Vec<Buffer>,
    label_positions: Vec<(f32, f32)>, // (x, y) in pixel coordinates

    // Additional labels (date and LOD)
    date_label_buffer: Option<Buffer>,
    date_label_position: (f32, f32),
    lod_label_buffer: Option<Buffer>,
    lod_label_position: (f32, f32),
    title_label_buffer: Option<Buffer>,
    title_label_position: (f32, f32),

    // Price grid lines and labels
    price_label_buffers: Vec<Buffer>,
    price_label_positions: Vec<(f32, f32)>,
    price_spacing: Option<PriceTickSpacing>,

    // Dividend lines and labels
    dividend_label_buffers: Vec<Buffer>,
    dividend_label_positions: Vec<(f32, f32)>,

    // Viewport info for text rendering
    viewport_x: f32,
    viewport_y: f32,
    viewport_w: f32,
    viewport_h: f32,

    // Visibility state for toggling
    visible: bool,

    // Theme colors
    grid_line_color: [f32; 4],
    text_primary_color: [u8; 4],
    text_secondary_color: [u8; 4],
}

impl GridRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        viewport_x: f32,
        viewport_y: f32,
        viewport_w: f32,
        viewport_h: f32,
        window_w: f32,
        window_h: f32,
        grid_line_color: [f32; 4],
        text_primary_color: [u8; 4],
        text_secondary_color: [u8; 4],
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Grid Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/grid.wgsl").into()),
        });

        let uniform = GridUniform {
            viewport: [viewport_x, viewport_y, viewport_w, viewport_h],
            window: [window_w, window_h, 0.0, 0.0],
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Grid Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Grid Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Grid Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Grid Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GridVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
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
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Create vertex buffer sized for ~100 lines max = 200 vertices
        let max_grid_vertices = 200;
        let grid_vertex_buffer_size =
            (max_grid_vertices * std::mem::size_of::<GridVertex>()) as u64;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Grid Vertex Buffer"),
            size: grid_vertex_buffer_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
            pipeline,
            uniform_buffer,
            bind_group,
            vertex_buffer,
            vertex_count: 0,

            font_system,
            swash_cache,
            text_atlas,
            text_renderer,

            label_buffers: Vec::new(),
            label_positions: Vec::new(),

            date_label_buffer: None,
            date_label_position: (0.0, 0.0),
            lod_label_buffer: None,
            lod_label_position: (0.0, 0.0),
            title_label_buffer: None,
            title_label_position: (0.0, 0.0),

            price_label_buffers: Vec::new(),
            price_label_positions: Vec::new(),
            price_spacing: None,

            dividend_label_buffers: Vec::new(),
            dividend_label_positions: Vec::new(),

            viewport_x,
            viewport_y,
            viewport_w,
            viewport_h,

            visible: true,

            grid_line_color,
            text_primary_color,
            text_secondary_color,
        }
    }

    pub fn update_uniforms(
        &mut self,
        queue: &wgpu::Queue,
        viewport_x: f32,
        viewport_y: f32,
        viewport_w: f32,
        viewport_h: f32,
        window_w: f32,
        window_h: f32,
    ) {
        self.viewport_x = viewport_x;
        self.viewport_y = viewport_y;
        self.viewport_w = viewport_w;
        self.viewport_h = viewport_h;

        let uniform = GridUniform {
            viewport: [viewport_x, viewport_y, viewport_w, viewport_h],
            window: [window_w, window_h, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    /// Compute and update grid lines based on visible candles and LOD level
    pub fn update_grid_lines(
        &mut self,
        queue: &wgpu::Queue,
        candles: &[lod::PlotCandle],
        lod_level: LodLevel,
        min_price: f32,
        max_price: f32,
        num_bars_in_viewport: u32,
        dividends: &[DividendWithIndex],
        start_idx: usize,
        ticker: Option<&str>,
        title: Option<&str>,
    ) {
        // Skip computation when grid is hidden
        if !self.visible {
            self.vertex_count = 0;
            self.label_buffers.clear();
            self.label_positions.clear();
            self.date_label_buffer = None;
            self.lod_label_buffer = None;
            self.title_label_buffer = None;
            self.price_label_buffers.clear();
            self.price_label_positions.clear();
            self.dividend_label_buffers.clear();
            self.dividend_label_positions.clear();
            return;
        }

        if candles.is_empty() {
            self.vertex_count = 0;
            self.label_buffers.clear();
            self.label_positions.clear();
            self.price_label_buffers.clear();
            self.price_label_positions.clear();
            self.dividend_label_buffers.clear();
            self.dividend_label_positions.clear();
            self.title_label_buffer = None;
            return;
        }

        let viewport_bars = num_bars_in_viewport.max(1) as f32;
        let mut vertices = Vec::new();
        let mut grid_lines: Vec<(f32, i64)> = Vec::new(); // (x_normalized, timestamp_secs)

        // First, generate horizontal price grid lines
        let price_span = max_price - min_price;
        if price_span > 0.0 && price_span.is_finite() {
            const MIN_PRICE_TICKS: usize = 4;
            const MAX_PRICE_TICKS: usize = 10;

            let spacing = select_price_spacing(
                price_span,
                self.price_spacing,
                MIN_PRICE_TICKS,
                MAX_PRICE_TICKS,
            );
            self.price_spacing = Some(spacing);

            let price_ticks = spacing.generate_ticks(min_price, max_price);

            for &price in &price_ticks {
                // Normalize price to 0.0..1.0 in viewport coordinates
                // Need to invert: 0.0 = max_price (top), 1.0 = min_price (bottom)
                // This matches how labels calculate position with (1.0 - y_norm)
                let y_norm = (max_price - price) / price_span;

                // Add horizontal line (from left to right edge of viewport)
                vertices.push(GridVertex {
                    position: [0.0, y_norm],
                    color: self.grid_line_color,
                });
                vertices.push(GridVertex {
                    position: [1.0, y_norm],
                    color: self.grid_line_color,
                });
            }

            // Create price labels
            self.create_price_labels(&price_ticks, &spacing, min_price, max_price);
        } else {
            self.price_label_buffers.clear();
            self.price_label_positions.clear();
        }

        // Then, determine what kind of vertical time grid lines to draw based on LOD level
        // Target density: 30-60 bars per grid line
        match lod_level {
            // S1 (1s bars): mark every minute (60 bars per line)
            LodLevel::S1 => {
                let mut last_minute = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);
                    let minute = (dt_et.hour(), dt_et.minute());

                    if last_minute != Some(minute) && dt_et.second() == 0 {
                        if idx != 0 {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                        }
                        last_minute = Some(minute);
                    }
                }
            }

            // S5 (5s bars): mark every 5 minutes (60 bars per line)
            LodLevel::S5 => {
                let mut last_mark = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);

                    if dt_et.minute() % 5 == 0 && dt_et.second() == 0 {
                        let mark = (dt_et.hour(), dt_et.minute());
                        if last_mark != Some(mark) && idx != 0 {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                            last_mark = Some(mark);
                        }
                    }
                }
            }

            // S15 (15s bars): mark every 15 minutes (60 bars per line)
            LodLevel::S15 => {
                let mut last_mark = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);

                    if dt_et.minute() % 15 == 0 && dt_et.second() == 0 {
                        let mark = (dt_et.hour(), dt_et.minute());
                        if last_mark != Some(mark) && idx != 0 {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                            last_mark = Some(mark);
                        }
                    }
                }
            }

            // S30 (30s bars): mark every 30 minutes (60 bars per line)
            LodLevel::S30 => {
                let mut last_mark = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);

                    if dt_et.minute() % 30 == 0 && dt_et.second() == 0 {
                        let mark = (dt_et.hour(), dt_et.minute());
                        if last_mark != Some(mark) && idx != 0 {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                            last_mark = Some(mark);
                        }
                    }
                }
            }

            // M1 (1min bars): mark every hour (60 bars per line) + 9:30am market open
            LodLevel::M1 => {
                let mut last_hour = None;
                let mut last_open_day = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);
                    let hour = dt_et.hour();
                    let day = (dt_et.year(), dt_et.month(), dt_et.day());

                    let is_market_open =
                        dt_et.hour() == 9 && dt_et.minute() == 30 && dt_et.second() == 0;

                    let is_hour_boundary = dt_et.minute() == 0 && dt_et.second() == 0;

                    let should_draw_hour = is_hour_boundary && last_hour != Some(hour);
                    let should_draw_open = is_market_open && last_open_day != Some(day);

                    if should_draw_hour || should_draw_open {
                        if idx != 0 || should_draw_open {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                        }
                        if should_draw_hour {
                            last_hour = Some(hour);
                        }
                        if should_draw_open {
                            last_open_day = Some(day);
                        }
                    }
                }
            }

            // M5 (5min bars): mark each trading day (78 bars per day)
            LodLevel::M5 => {
                let mut last_day = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);
                    let day = (dt_et.year(), dt_et.month(), dt_et.day());

                    if last_day != Some(day) {
                        if idx != 0 {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                        }
                        last_day = Some(day);
                    }
                }
            }

            // M15 (15min bars): mark each trading day (26 bars per day)
            // This is on the low side, but marking every week would be too sparse
            LodLevel::M15 => {
                let mut last_day = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);
                    let day = (dt_et.year(), dt_et.month(), dt_et.day());

                    if last_day != Some(day) {
                        if idx != 0 {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                        }
                        last_day = Some(day);
                    }
                }
            }

            // For 30min and 1h bars, mark start of each week (Monday)
            LodLevel::M30 | LodLevel::H1 => {
                let mut last_week = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);

                    // Get ISO week number (week starts on Monday)
                    let week = dt_et.iso_week().week();
                    let year = dt_et.iso_week().year();
                    let week_id = (year, week);

                    if last_week != Some(week_id) {
                        if idx != 0 {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                        }
                        last_week = Some(week_id);
                    }
                }
            }

            // For 4h bars, mark start of each month
            LodLevel::H4 => {
                let mut last_month = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);
                    let month = dt_et.month();

                    if last_month != Some(month) {
                        if idx != 0 {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                        }
                        last_month = Some(month);
                    }
                }
            }

            // For daily bars, mark start of each month
            LodLevel::D1 => {
                let mut last_month = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);
                    let month = dt_et.month();

                    if last_month != Some(month) {
                        if idx != 0 {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                        }
                        last_month = Some(month);
                    }
                }
            }

            // For weekly/monthly bars, mark start of each year
            LodLevel::W1 | LodLevel::Month1 => {
                let mut last_year = None;
                for (idx, candle) in candles.iter().enumerate() {
                    let dt_utc = DateTime::from_timestamp_nanos(candle.ts);
                    let dt_et = dt_utc.with_timezone(&New_York);
                    let year = dt_et.year();

                    if idx != 0 {
                        if last_year != Some(year) {
                            let x = idx as f32 / viewport_bars;
                            vertices.push(GridVertex {
                                position: [x, 0.0],
                                color: self.grid_line_color,
                            });
                            vertices.push(GridVertex {
                                position: [x, 1.0],
                                color: self.grid_line_color,
                            });
                            grid_lines.push((x, candle.ts / 1_000_000_000));
                        }
                        last_year = Some(year);
                    }
                }
            }
        }

        // Extend grid lines into empty space beyond the last candle.
        // The candle-based loop above only places lines where data exists;
        // this fills the remaining viewport with the same boundary pattern.
        if !candles.is_empty() && candles.len() < num_bars_in_viewport as usize {
            let lod_secs = lod_level.seconds() as i64;
            let last_candle_ts_ns = candles.last().unwrap().ts;
            let last_candle_idx = candles.len() - 1;
            let remaining_bars = num_bars_in_viewport as usize - candles.len();

            {
                for extra in 1..=remaining_bars {
                    let idx = last_candle_idx + extra;
                    let ts_ns = last_candle_ts_ns + (extra as i64) * lod_secs * 1_000_000_000;
                    let dt_utc = DateTime::from_timestamp_nanos(ts_ns);
                    let dt_et = dt_utc.with_timezone(&New_York);

                    let is_boundary = match lod_level {
                        LodLevel::S1 => dt_et.second() == 0,
                        LodLevel::S5 => dt_et.minute() % 5 == 0 && dt_et.second() == 0,
                        LodLevel::S15 => dt_et.minute() % 15 == 0 && dt_et.second() == 0,
                        LodLevel::S30 => dt_et.minute() % 30 == 0 && dt_et.second() == 0,
                        LodLevel::M1 => {
                            let is_hour = dt_et.minute() == 0 && dt_et.second() == 0;
                            let is_open = dt_et.hour() == 9 && dt_et.minute() == 30 && dt_et.second() == 0;
                            is_hour || is_open
                        }
                        LodLevel::M5 | LodLevel::M15 => {
                            // Day boundary: check if this is the first bar of a new day
                            let prev_ts_ns = ts_ns - lod_secs * 1_000_000_000;
                            let prev_dt = DateTime::from_timestamp_nanos(prev_ts_ns).with_timezone(&New_York);
                            dt_et.day() != prev_dt.day()
                        }
                        LodLevel::M30 | LodLevel::H1 => {
                            let prev_ts_ns = ts_ns - lod_secs * 1_000_000_000;
                            let prev_dt = DateTime::from_timestamp_nanos(prev_ts_ns).with_timezone(&New_York);
                            dt_et.iso_week().week() != prev_dt.iso_week().week()
                                || dt_et.iso_week().year() != prev_dt.iso_week().year()
                        }
                        LodLevel::H4 | LodLevel::D1 => {
                            let prev_ts_ns = ts_ns - lod_secs * 1_000_000_000;
                            let prev_dt = DateTime::from_timestamp_nanos(prev_ts_ns).with_timezone(&New_York);
                            dt_et.month() != prev_dt.month()
                        }
                        LodLevel::W1 | LodLevel::Month1 => {
                            let prev_ts_ns = ts_ns - lod_secs * 1_000_000_000;
                            let prev_dt = DateTime::from_timestamp_nanos(prev_ts_ns).with_timezone(&New_York);
                            dt_et.year() != prev_dt.year()
                        }
                    };

                    if is_boundary {
                        let x = idx as f32 / viewport_bars;
                        vertices.push(GridVertex {
                            position: [x, 0.0],
                            color: self.grid_line_color,
                        });
                        vertices.push(GridVertex {
                            position: [x, 1.0],
                            color: self.grid_line_color,
                        });
                        grid_lines.push((x, ts_ns / 1_000_000_000));
                    }
                }
            }
        }

        // Add dividend lines
        let interval_secs = lod_level.seconds() as u64;
        let end_idx = start_idx + candles.len();
        for dividend in dividends {
            // Look up precomputed index for current LOD level
            if let Some(&bar_index) = dividend.indices.get(&interval_secs) {
                // Check if dividend is in visible range
                if bar_index >= start_idx && bar_index < end_idx {
                    // Calculate position RELATIVE to visible slice
                    let relative_index = bar_index - start_idx;
                    let x_norm = relative_index as f32 / viewport_bars;

                    // Add vertical green line
                    vertices.push(GridVertex {
                        position: [x_norm, 0.0],
                        color: [0.0, 1.0, 0.0, 0.8],
                    });
                    vertices.push(GridVertex {
                        position: [x_norm, 1.0],
                        color: [0.0, 1.0, 0.0, 0.8],
                    });
                }
            }
        }

        // Limit to buffer size (200 vertices = 100 lines)
        if vertices.len() > 200 {
            vertices.truncate(200);
            grid_lines.truncate(100);
        }

        self.vertex_count = vertices.len() as u32;

        if !vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }

        // Create label buffers and positions
        self.create_label_buffers(&grid_lines, lod_level);

        // Create date and LOD labels
        self.create_special_labels(candles, lod_level, ticker, title);

        // Create dividend labels
        self.create_dividend_labels(dividends, interval_secs, viewport_bars, start_idx, end_idx);
    }

    fn create_label_buffers(&mut self, grid_lines: &[(f32, i64)], lod_level: LodLevel) {
        // Clear previous labels
        self.label_buffers.clear();
        self.label_positions.clear();

        if grid_lines.is_empty() {
            return;
        }

        // Derive format string from LOD level (matching grid line granularity)
        let format_str = match lod_level {
            // Time of day for intraday minute/second intervals
            LodLevel::S1 | LodLevel::S5 | LodLevel::S15 | LodLevel::S30 | LodLevel::M1 => "%H:%M",
            // Day of week for multi-hour intraday intervals
            LodLevel::M5 | LodLevel::M15 => "%a",
            // Day + time for multi-day intervals
            LodLevel::M30 | LodLevel::H1 => "%a %H:%M",
            // Month for long-term daily/4h bars
            LodLevel::H4 | LodLevel::D1 => "%b",
            // Year for weekly/monthly bars
            LodLevel::W1 | LodLevel::Month1 => "%Y",
        };

        for &(x_norm, timestamp_secs) in grid_lines {
            // Format label directly using format_str
            let dt = DateTime::<chrono::Utc>::from_timestamp(timestamp_secs, 0)
                .map(|d| d.with_timezone(&New_York));

            let label_text = if let Some(dt_et) = dt {
                dt_et.format(format_str).to_string()
            } else {
                continue;
            };

            // Calculate pixel position
            // x_norm is 0.0 to 1.0 in the viewport
            let x_pixel = self.viewport_x + x_norm * self.viewport_w;

            // Estimate label width (monospace ~8.5px per char)
            let label_width = label_text.len() as f32 * 8.5;
            let label_right = x_pixel + label_width;

            // Cull labels that would extend beyond viewport right edge
            if label_right > self.viewport_x + self.viewport_w {
                continue;
            }

            // Create a text buffer
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
            buffer.set_text(
                &mut self.font_system,
                &label_text,
                Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
            );

            // Place label at bottom of viewport, with small offset above
            let y_pixel = self.viewport_y + self.viewport_h - 20.0;

            self.label_buffers.push(buffer);
            self.label_positions.push((x_pixel, y_pixel));
        }
    }

    fn create_special_labels(
        &mut self,
        candles: &[lod::PlotCandle],
        lod_level: LodLevel,
        ticker: Option<&str>,
        title: Option<&str>,
    ) {
        // Clear previous special labels
        self.date_label_buffer = None;
        self.lod_label_buffer = None;
        self.title_label_buffer = None;

        // Always create LOD label (top left)
        // Format: "TICKER | LOD" if ticker is available, otherwise just "LOD"
        let lod_text = if let Some(ticker) = ticker {
            format!("{} | {}", ticker, lod_level.label())
        } else {
            lod_level.label().to_string()
        };
        let mut lod_buffer = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
        lod_buffer.set_text(
            &mut self.font_system,
            &lod_text,
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        self.lod_label_buffer = Some(lod_buffer);
        self.lod_label_position = (self.viewport_x + 5.0, self.viewport_y + 5.0);

        if let Some(title_text) = title {
            let mut title_buffer = Buffer::new(&mut self.font_system, Metrics::new(16.0, 20.0));
            title_buffer.set_text(
                &mut self.font_system,
                title_text,
                Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
            );

            let approx_width = title_text.chars().count() as f32 * 9.0;
            let x_center = self.viewport_x + ((self.viewport_w - approx_width).max(0.0)) * 0.5;
            let y_top = self.viewport_y + 5.0;

            self.title_label_buffer = Some(title_buffer);
            self.title_label_position = (x_center, y_top);
        }

        // Create date label only for S1 to H1
        let should_show_date = matches!(
            lod_level,
            LodLevel::S1
                | LodLevel::S5
                | LodLevel::S15
                | LodLevel::S30
                | LodLevel::M1
                | LodLevel::M5
                | LodLevel::M15
                | LodLevel::M30
                | LodLevel::H1
        );

        if should_show_date && !candles.is_empty() {
            // Get the first visible candle's timestamp
            let first_candle = &candles[0];
            let dt_utc = DateTime::from_timestamp_nanos(first_candle.ts);
            let dt_et = dt_utc.with_timezone(&New_York);

            // Format as dd/mm/yy
            let date_text = format!(
                "{:02}/{:02}/{:02}",
                dt_et.day(),
                dt_et.month(),
                dt_et.year() % 100
            );

            let mut date_buffer = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
            date_buffer.set_text(
                &mut self.font_system,
                &date_text,
                Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
            );
            self.date_label_buffer = Some(date_buffer);
            // Position at bottom left, just above the tick labels (40 pixels from bottom)
            self.date_label_position = (
                self.viewport_x + 3.0,
                self.viewport_y + self.viewport_h - 40.0,
            );
        }
    }

    fn create_price_labels(
        &mut self,
        price_ticks: &[f32],
        spacing: &PriceTickSpacing,
        min_price: f32,
        max_price: f32,
    ) {
        self.price_label_buffers.clear();
        self.price_label_positions.clear();

        let price_span = max_price - min_price;
        if price_span <= 0.0 || !price_span.is_finite() {
            return;
        }

        // Label height is 18.0 pixels (from Metrics)
        const LABEL_HEIGHT: f32 = 18.0;

        for &price in price_ticks {
            let label_text = spacing.format_price(price);

            // Calculate pixel position
            // Normalize price to viewport coordinates (0.0 to 1.0)
            let y_norm = (price - min_price) / price_span;
            // Convert to pixel position (invert y-axis as screen coordinates go top-to-bottom)
            let y_pixel = self.viewport_y + self.viewport_h * (1.0 - y_norm) + 7.0;

            // Calculate final label top position
            let label_top = y_pixel - 7.0;
            let label_bottom = label_top + LABEL_HEIGHT;

            // Cull labels that would extend beyond viewport bottom
            if label_bottom > self.viewport_y + self.viewport_h {
                continue;
            }

            // Create text buffer
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
            buffer.set_text(
                &mut self.font_system,
                &label_text,
                Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
            );

            // Estimate text width for right-justification (monospace ~8.5px per char)
            let text_width = label_text.len() as f32 * 8.5;
            // Position so text ends at viewport right edge with 5px padding
            let x_pixel = self.viewport_x + self.viewport_w - text_width - 5.0;

            self.price_label_buffers.push(buffer);
            self.price_label_positions.push((x_pixel, label_top));
        }
    }

    fn create_dividend_labels(
        &mut self,
        dividends: &[DividendWithIndex],
        interval_secs: u64,
        viewport_bars: f32,
        start_idx: usize,
        end_idx: usize,
    ) {
        self.dividend_label_buffers.clear();
        self.dividend_label_positions.clear();

        for dividend in dividends {
            if let Some(&bar_index) = dividend.indices.get(&interval_secs) {
                // Check if dividend is in visible range
                if bar_index >= start_idx && bar_index < end_idx {
                    let relative_index = bar_index - start_idx;
                    let x_norm = relative_index as f32 / viewport_bars;
                    let label_text = format!("${:.2}", dividend.event.amount);

                    let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
                    buffer.set_text(
                        &mut self.font_system,
                        &label_text,
                        Attrs::new().family(Family::Monospace),
                        Shaping::Advanced,
                    );

                    let x_pixel = self.viewport_x + x_norm * self.viewport_w;
                    let y_pixel = self.viewport_y + 3.0; // Near top

                    self.dividend_label_buffers.push(buffer);
                    self.dividend_label_positions.push((x_pixel, y_pixel));
                }
            }
        }
    }

    pub fn draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        if !self.visible || self.vertex_count == 0 {
            return;
        }

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..1);
    }

    pub fn draw_labels<'a>(
        &'a mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_pass: &mut wgpu::RenderPass<'a>,
        viewport: &Viewport,
    ) {
        if !self.visible {
            return;
        }

        // Prepare text areas for grid labels
        let mut text_areas = Vec::new();

        // Add time grid labels (bottom)
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
                default_color: Color::rgba(
                    self.text_secondary_color[0],
                    self.text_secondary_color[1],
                    self.text_secondary_color[2],
                    self.text_secondary_color[3],
                ),
                custom_glyphs: &[],
            });
        }

        // Add price grid labels (right side)
        for (i, (x, y)) in self.price_label_positions.iter().enumerate() {
            text_areas.push(TextArea {
                buffer: &self.price_label_buffers[i],
                left: *x,
                top: *y,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: i32::MAX,
                    bottom: i32::MAX,
                },
                default_color: Color::rgba(
                    self.text_secondary_color[0],
                    self.text_secondary_color[1],
                    self.text_secondary_color[2],
                    self.text_secondary_color[3],
                ),
                custom_glyphs: &[],
            });
        }

        // Add dividend labels (top, green)
        for (i, (x, y)) in self.dividend_label_positions.iter().enumerate() {
            text_areas.push(TextArea {
                buffer: &self.dividend_label_buffers[i],
                left: *x,
                top: *y,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: i32::MAX,
                    bottom: i32::MAX,
                },
                default_color: Color::rgb(0, 255, 0),
                custom_glyphs: &[],
            });
        }

        // Add LOD label (top left)
        if let Some(ref lod_buffer) = self.lod_label_buffer {
            text_areas.push(TextArea {
                buffer: lod_buffer,
                left: self.lod_label_position.0,
                top: self.lod_label_position.1,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: i32::MAX,
                    bottom: i32::MAX,
                },
                default_color: Color::rgba(
                    self.text_primary_color[0],
                    self.text_primary_color[1],
                    self.text_primary_color[2],
                    self.text_primary_color[3],
                ),
                custom_glyphs: &[],
            });
        }

        // Add title label (top center)
        if let Some(ref title_buffer) = self.title_label_buffer {
            text_areas.push(TextArea {
                buffer: title_buffer,
                left: self.title_label_position.0,
                top: self.title_label_position.1,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: i32::MAX,
                    bottom: i32::MAX,
                },
                default_color: Color::rgba(
                    self.text_primary_color[0],
                    self.text_primary_color[1],
                    self.text_primary_color[2],
                    self.text_primary_color[3],
                ),
                custom_glyphs: &[],
            });
        }

        // Add date label (bottom left) if present
        if let Some(ref date_buffer) = self.date_label_buffer {
            text_areas.push(TextArea {
                buffer: date_buffer,
                left: self.date_label_position.0,
                top: self.date_label_position.1,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: i32::MAX,
                    bottom: i32::MAX,
                },
                default_color: Color::rgba(
                    self.text_primary_color[0],
                    self.text_primary_color[1],
                    self.text_primary_color[2],
                    self.text_primary_color[3],
                ),
                custom_glyphs: &[],
            });
        }

        // Prepare and render all text areas
        if !text_areas.is_empty() {
            self.text_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.text_atlas,
                    viewport,
                    text_areas,
                    &mut self.swash_cache,
                )
                .ok();

            self.text_renderer
                .render(&self.text_atlas, viewport, render_pass)
                .ok();
        }
    }

    /// Toggle grid visibility
    pub fn toggle_visibility(&mut self) {
        self.visible = !self.visible;
    }

    /// Get current visibility state
    pub fn is_visible(&self) -> bool {
        self.visible
    }
}
