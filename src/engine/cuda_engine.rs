use std::sync::Arc;
use std::time::Instant;
use cudarc::driver::{CudaContext, CudaFunction, CudaSlice, CudaStream, LaunchConfig, PushKernelArg};
use cudarc::nvrtc::compile_ptx;
use crate::agent::state::AgentState;
use crate::market::types::{BBO, Order};
use super::{GpuStepTimings, SimEngine};

const KERNEL_SRC: &str = include_str!("../../kernels/decide.cu");
const TEMPLATE_SRC: &str = include_str!("../../kernels/decide_template.cu");

/// Default signal expression matching the hardcoded logic in decide.cu.
#[allow(dead_code)]
const DEFAULT_SIGNAL_EXPR: &str =
    "(fair_value_estimate - mid) * mean_reversion + (mid - ema) * trend_follow + noise + (-risk_aversion * pos)";

pub struct CudaEngine {
    ctx: Arc<CudaContext>,
    stream: Arc<CudaStream>,
    d_position: CudaSlice<f32>,
    d_cash: CudaSlice<f32>,
    d_strategy_params: CudaSlice<f32>,
    d_internal_state: CudaSlice<f32>,
    d_order_price: CudaSlice<f32>,
    d_order_quantity: CudaSlice<f32>,
    func: CudaFunction,
    n: usize,
    k: usize,
    m: usize,
    block_size: u32,
    // Pre-allocated host staging buffers reused every tick
    h_pos_f32: Vec<f32>,
    h_prices: Vec<f32>,
    h_qtys: Vec<f32>,
}

impl CudaEngine {
    /// Substitute `signal_expr` into the kernel template and compile via NVRTC.
    pub fn compile_kernel(ctx: &Arc<CudaContext>, template: &str, signal_expr: &str) -> Result<CudaFunction, Box<dyn std::error::Error>> {
        let src = template.replace("{{SIGNAL_EXPR}}", signal_expr);
        let ptx = compile_ptx(&src)?;
        let module = ctx.load_module(ptx)?;
        let func = module.load_function("agent_decide")?;
        Ok(func)
    }

    pub fn new(device_id: usize, agents: &AgentState, signal_expr: Option<&str>) -> Result<Self, Box<dyn std::error::Error>> {
        let ctx = CudaContext::new(device_id)?;
        let stream = ctx.default_stream();

        let func = match signal_expr {
            Some(expr) => Self::compile_kernel(&ctx, TEMPLATE_SRC, expr)?,
            None => {
                // Use the original kernel for exact backward compatibility
                let ptx = compile_ptx(KERNEL_SRC)?;
                let module = ctx.load_module(ptx)?;
                module.load_function("agent_decide")?
            }
        };

        let n = agents.n;
        let k = agents.k;
        let m = agents.m;

        // Upload initial state: cast position/cash f64->f32
        let pos_f32: Vec<f32> = agents.position.iter().map(|&x| x as f32).collect();
        let cash_f32: Vec<f32> = agents.cash.iter().map(|&x| x as f32).collect();

        let d_position = stream.memcpy_stod(&pos_f32)?;
        let d_cash = stream.memcpy_stod(&cash_f32)?;
        let d_strategy_params = stream.memcpy_stod(agents.strategy_params.as_slice())?;
        let d_internal_state = stream.memcpy_stod(agents.internal_state.as_slice())?;
        let d_order_price = stream.alloc_zeros::<f32>(n)?;
        let d_order_quantity = stream.alloc_zeros::<f32>(n)?;

        stream.synchronize()?;

        Ok(Self {
            ctx,
            stream,
            d_position,
            d_cash,
            d_strategy_params,
            d_internal_state,
            d_order_price,
            d_order_quantity,
            func,
            n,
            k,
            m,
            block_size: 256,
            h_pos_f32: vec![0.0f32; n],
            h_prices: vec![0.0f32; n],
            h_qtys: vec![0.0f32; n],
        })
    }

    pub fn upload_agents(&mut self, agents: &AgentState) -> Result<(), Box<dyn std::error::Error>> {
        let pos_f32: Vec<f32> = agents.position.iter().map(|&x| x as f32).collect();
        let cash_f32: Vec<f32> = agents.cash.iter().map(|&x| x as f32).collect();
        self.stream.memcpy_htod(&pos_f32, &mut self.d_position)?;
        self.stream.memcpy_htod(&cash_f32, &mut self.d_cash)?;
        self.stream.memcpy_htod(agents.strategy_params.as_slice(), &mut self.d_strategy_params)?;
        self.stream.memcpy_htod(agents.internal_state.as_slice(), &mut self.d_internal_state)?;
        Ok(())
    }

    pub fn download_agents(&self, agents: &mut AgentState) -> Result<(), Box<dyn std::error::Error>> {
        let internal_state = self.stream.memcpy_dtov(&self.d_internal_state)?;
        self.stream.synchronize()?;
        agents.internal_state.copy_from_slice(&internal_state);
        Ok(())
    }
}

impl SimEngine for CudaEngine {
    fn step(
        &mut self,
        agents: &mut AgentState,
        bbo: &BBO,
        order_buffer: &mut Vec<Order>,
    ) -> (usize, GpuStepTimings) {
        // --- Phase 5: GPU upload ---
        let t0 = Instant::now();
        for (dst, &src) in self.h_pos_f32.iter_mut().zip(agents.position.iter()) {
            *dst = src as f32;
        }
        self.stream.memcpy_htod(&self.h_pos_f32, &mut self.d_position).unwrap();
        let upload_time = t0.elapsed();

        // Compute grid dimensions
        let grid = (self.n as u32 + self.block_size - 1) / self.block_size;
        let cfg = LaunchConfig {
            grid_dim: (grid, 1, 1),
            block_dim: (self.block_size, 1, 1),
            shared_mem_bytes: 0,
        };

        // Kernel scalar args must be stack locals (PushKernelArg takes &T)
        let best_bid = bbo.best_bid;
        let best_ask = bbo.best_ask;
        let last_price = bbo.last_price;
        let fair_value = bbo.fair_value;
        let n_i32 = self.n as i32;
        let k_i32 = self.k as i32;
        let m_i32 = self.m as i32;

        // --- Phase 6: GPU kernel launch ---
        let t1 = Instant::now();
        unsafe {
            self.stream
                .launch_builder(&self.func)
                .arg(&best_bid)
                .arg(&best_ask)
                .arg(&last_price)
                .arg(&fair_value)
                .arg(&self.d_position)
                .arg(&self.d_cash)
                .arg(&self.d_strategy_params)
                .arg(&mut self.d_internal_state)
                .arg(&mut self.d_order_price)
                .arg(&mut self.d_order_quantity)
                .arg(&n_i32)
                .arg(&k_i32)
                .arg(&m_i32)
                .launch(cfg)
        }
        .unwrap();
        let kernel_time = t1.elapsed();

        // --- Phase 7: GPU download ---
        let t2 = Instant::now();
        self.stream.memcpy_dtoh(&self.d_order_price, &mut self.h_prices).unwrap();
        self.stream.memcpy_dtoh(&self.d_order_quantity, &mut self.h_qtys).unwrap();

        // Block until all async GPU work is complete before using the results
        self.stream.synchronize().unwrap();
        let download_time = t2.elapsed();

        // Build order buffer
        order_buffer.clear();
        order_buffer.reserve(self.n);
        for i in 0..self.n {
            order_buffer.push(Order {
                agent_id: i as u32,
                price: self.h_prices[i],
                quantity: self.h_qtys[i],
            });
        }

        let gpu_timings = GpuStepTimings {
            upload: upload_time,
            kernel: kernel_time,
            download: download_time,
        };
        (self.n, gpu_timings)
    }

    fn reload_kernel(&mut self, signal_expr: &str) -> Result<(), Box<dyn std::error::Error>> {
        let func = Self::compile_kernel(&self.ctx, TEMPLATE_SRC, signal_expr)?;
        self.func = func;
        Ok(())
    }
}
