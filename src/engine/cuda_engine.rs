use std::sync::Arc;
use std::time::Instant;
use cudarc::driver::{CudaContext, CudaFunction, CudaSlice, CudaStream, DevicePtr, DeviceRepr, HostSlice, LaunchConfig, PinnedHostSlice, PushKernelArg, SyncOnDrop, ValidAsZeroBits};
use cudarc::nvrtc::compile_ptx;
use crate::agent::state::AgentState;
use crate::market::types::{BBO, LobOrder, OrderType, Side};
use super::{GpuStepTimings, SimEngine};

/// Page-locked (pinned) host memory **without** `CU_MEMHOSTALLOC_WRITECOMBINED`.
///
/// `PinnedHostSlice` (cudarc's built-in) always sets the WC flag, which makes CPU
/// reads slow (bypasses L1/L2 cache). This wrapper uses flags=0 so the memory is
/// cache-coherent — correct for DtoH transfers where the CPU reads results immediately.
struct PinnedReadableSlice<T> {
    ptr: *mut T,
    len: usize,
    // Keeps the context alive until after free_host runs. Rust drops fields in
    // declaration order, so ctx must be listed LAST to outlive ptr.
    ctx: Arc<CudaContext>,
}

unsafe impl<T: Send> Send for PinnedReadableSlice<T> {}
unsafe impl<T: Sync> Sync for PinnedReadableSlice<T> {}

impl<T> PinnedReadableSlice<T> {
    fn new(ctx: &Arc<CudaContext>, len: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let ptr = unsafe {
            cudarc::driver::result::malloc_host(len * std::mem::size_of::<T>(), 0)?
        };
        Ok(Self { ptr: ptr as *mut T, len, ctx: ctx.clone() })
    }

    fn as_slice(&self) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<T> Drop for PinnedReadableSlice<T> {
    fn drop(&mut self) {
        // ctx Arc ensures the CUDA context is still alive here.
        // record_err swallows errors rather than panicking in a destructor.
        self.ctx.record_err(unsafe {
            cudarc::driver::result::free_host(self.ptr as *mut std::ffi::c_void)
        });
    }
}

impl<T> HostSlice<T> for PinnedReadableSlice<T> {
    fn len(&self) -> usize {
        self.len
    }
    // SyncOnDrop::Sync(None) is a no-op, same as the Vec<T> impl in cudarc.
    // Explicit stream.synchronize() in step() covers our synchronization needs.
    unsafe fn stream_synced_slice<'a>(
        &'a self,
        _stream: &'a CudaStream,
    ) -> (&'a [T], SyncOnDrop<'a>) {
        (std::slice::from_raw_parts(self.ptr, self.len), SyncOnDrop::Sync(None))
    }
    unsafe fn stream_synced_mut_slice<'a>(
        &'a mut self,
        _stream: &'a CudaStream,
    ) -> (&'a mut [T], SyncOnDrop<'a>) {
        (std::slice::from_raw_parts_mut(self.ptr, self.len), SyncOnDrop::Sync(None))
    }
}

const KERNEL_SRC: &str = include_str!("../../kernels/decide.cu");
const TEMPLATE_SRC: &str = include_str!("../../kernels/decide_template.cu");
const CLASSIFY_SRC: &str = include_str!("../../kernels/classify.cu");
const COMPACT_SRC: &str = include_str!("../../kernels/compact.cu");

/// Mirrors `struct CompactOrder` in kernels/compact.cu (must stay in sync).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CompactOrder {
    agent_id:    u32,
    order_type:  i32,
    bid_price:   f32,
    ask_price:   f32,
    qty:         f32,
    cancel_flag: i32,
}

// Safety: all fields are primitive types; zero is a valid bit pattern.
unsafe impl DeviceRepr for CompactOrder {}
unsafe impl ValidAsZeroBits for CompactOrder {}

/// Default signal expression matching the hardcoded logic in decide.cu.
#[allow(dead_code)]
const DEFAULT_SIGNAL_EXPR: &str =
    "(fair_value_estimate - mid) * mean_reversion + (mid - ema) * trend_follow + noise + (-risk_aversion * pos)";

pub struct CudaEngine {
    ctx: Arc<CudaContext>,
    stream: Arc<CudaStream>,
    // Agent state (GPU-resident)
    d_position: CudaSlice<f32>,
    d_cash: CudaSlice<f32>,
    d_strategy_params: CudaSlice<f32>,
    d_internal_state: CudaSlice<f32>,
    // Intermediate buffers: agent_decide writes, classify reads (never cross PCIe)
    d_order_price: CudaSlice<f32>,
    d_order_quantity: CudaSlice<f32>,
    // MM agent arrays (GPU-resident, uploaded once at init)
    d_agent_type: CudaSlice<i32>,
    d_mm_half_spread: CudaSlice<f32>,
    d_mm_quote_size: CudaSlice<f32>,
    d_mm_requote_threshold: CudaSlice<f32>,
    d_mm_last_quote_mid: CudaSlice<f32>,  // also written by classify kernel
    // Classified output (GPU)
    d_out_order_type: CudaSlice<i32>,
    d_out_bid_price: CudaSlice<f32>,
    d_out_ask_price: CudaSlice<f32>,
    d_out_qty: CudaSlice<f32>,
    d_out_cancel_flag: CudaSlice<i32>,
    // Kernels
    decide_func: CudaFunction,
    classify_func: CudaFunction,
    compact_func: CudaFunction,
    n: usize,
    k: usize,
    m: usize,
    block_size: u32,
    // Classification scalar uniforms
    participation_threshold: f32,
    market_order_threshold: f32,
    tick_size: f32,
    // Pre-allocated host staging buffers reused every tick
    h_pos_f32: PinnedHostSlice<f32>,           // WC pinned: CPU writes, GPU reads (upload)
    // Compacted output (dense, only active entries) — replaces full N-sized h_out_* buffers
    d_compact: CudaSlice<CompactOrder>,        // device: packed active entries (max N)
    d_active_count: CudaSlice<u32>,            // device: scalar count, zeroed before each tick
    h_compact: PinnedReadableSlice<CompactOrder>, // host: DtoH destination (max N entries)
    h_active_count: PinnedReadableSlice<u32>,  // host: DtoH destination for count scalar
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

    fn compile_classify(ctx: &Arc<CudaContext>) -> Result<CudaFunction, Box<dyn std::error::Error>> {
        let ptx = compile_ptx(CLASSIFY_SRC)?;
        let module = ctx.load_module(ptx)?;
        let func = module.load_function("classify_orders")?;
        Ok(func)
    }

    fn compile_compact(ctx: &Arc<CudaContext>) -> Result<CudaFunction, Box<dyn std::error::Error>> {
        let ptx = compile_ptx(COMPACT_SRC)?;
        let module = ctx.load_module(ptx)?;
        let func = module.load_function("compact_orders")?;
        Ok(func)
    }

    pub fn new(
        device_id: usize,
        agents: &AgentState,
        signal_expr: Option<&str>,
        participation_threshold: f32,
        market_order_threshold: f32,
        tick_size: f32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let ctx = CudaContext::new(device_id)?;
        let stream = ctx.default_stream();

        let decide_func = match signal_expr {
            Some(expr) => Self::compile_kernel(&ctx, TEMPLATE_SRC, expr)?,
            None => {
                // Use the original kernel for exact backward compatibility
                let ptx = compile_ptx(KERNEL_SRC)?;
                let module = ctx.load_module(ptx)?;
                module.load_function("agent_decide")?
            }
        };
        let classify_func = Self::compile_classify(&ctx)?;
        let compact_func = Self::compile_compact(&ctx)?;

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

        // Upload MM agent arrays (cast agent_type u8 -> i32 for GPU alignment)
        let agent_type_i32: Vec<i32> = agents.agent_type.iter().map(|&x| x as i32).collect();
        let d_agent_type = stream.memcpy_stod(&agent_type_i32)?;
        let d_mm_half_spread = stream.memcpy_stod(&agents.mm_half_spread)?;
        let d_mm_quote_size = stream.memcpy_stod(&agents.mm_quote_size)?;
        let d_mm_requote_threshold = stream.memcpy_stod(&agents.mm_requote_threshold)?;
        let d_mm_last_quote_mid = stream.memcpy_stod(&agents.mm_last_quote_mid)?;

        // Classified output GPU buffers
        let d_out_order_type = stream.alloc_zeros::<i32>(n)?;
        let d_out_bid_price = stream.alloc_zeros::<f32>(n)?;
        let d_out_ask_price = stream.alloc_zeros::<f32>(n)?;
        let d_out_qty = stream.alloc_zeros::<f32>(n)?;
        let d_out_cancel_flag = stream.alloc_zeros::<i32>(n)?;

        // Compact output buffers (device + host), sized to worst case N
        let d_compact = stream.alloc_zeros::<CompactOrder>(n)?;
        let d_active_count = stream.alloc_zeros::<u32>(1)?;

        stream.synchronize()?;

        Ok(Self {
            ctx: ctx.clone(),
            stream,
            d_position,
            d_cash,
            d_strategy_params,
            d_internal_state,
            d_order_price,
            d_order_quantity,
            d_agent_type,
            d_mm_half_spread,
            d_mm_quote_size,
            d_mm_requote_threshold,
            d_mm_last_quote_mid,
            d_out_order_type,
            d_out_bid_price,
            d_out_ask_price,
            d_out_qty,
            d_out_cancel_flag,
            decide_func,
            classify_func,
            compact_func,
            n,
            k,
            m,
            block_size: 256,
            participation_threshold,
            market_order_threshold,
            tick_size,
            h_pos_f32: {
                let mut buf = unsafe { ctx.alloc_pinned::<f32>(n) }?;
                let s = buf.as_mut_slice()?;
                for (dst, &src) in s.iter_mut().zip(agents.position.iter()) {
                    *dst = src as f32;
                }
                buf
            },
            d_compact,
            d_active_count,
            h_compact: PinnedReadableSlice::new(&ctx, n)?,
            h_active_count: PinnedReadableSlice::new(&ctx, 1)?,
        })
    }

    pub fn upload_agents(&mut self, agents: &AgentState) -> Result<(), Box<dyn std::error::Error>> {
        let pos_f32: Vec<f32> = agents.position.iter().map(|&x| x as f32).collect();
        let cash_f32: Vec<f32> = agents.cash.iter().map(|&x| x as f32).collect();
        self.stream.memcpy_htod(&pos_f32, &mut self.d_position)?;
        self.stream.memcpy_htod(&cash_f32, &mut self.d_cash)?;
        self.stream.memcpy_htod(agents.strategy_params.as_slice(), &mut self.d_strategy_params)?;
        self.stream.memcpy_htod(agents.internal_state.as_slice(), &mut self.d_internal_state)?;
        // Re-upload MM arrays
        let agent_type_i32: Vec<i32> = agents.agent_type.iter().map(|&x| x as i32).collect();
        self.stream.memcpy_htod(&agent_type_i32, &mut self.d_agent_type)?;
        self.stream.memcpy_htod(&agents.mm_half_spread, &mut self.d_mm_half_spread)?;
        self.stream.memcpy_htod(&agents.mm_quote_size, &mut self.d_mm_quote_size)?;
        self.stream.memcpy_htod(&agents.mm_requote_threshold, &mut self.d_mm_requote_threshold)?;
        self.stream.memcpy_htod(&agents.mm_last_quote_mid, &mut self.d_mm_last_quote_mid)?;
        Ok(())
    }

    pub fn download_agents(&self, agents: &mut AgentState) -> Result<(), Box<dyn std::error::Error>> {
        let internal_state = self.stream.memcpy_dtov(&self.d_internal_state)?;
        let mm_last_quote_mid = self.stream.memcpy_dtov(&self.d_mm_last_quote_mid)?;
        self.stream.synchronize()?;
        agents.internal_state.copy_from_slice(&internal_state);
        agents.mm_last_quote_mid.copy_from_slice(&mm_last_quote_mid);
        Ok(())
    }
}

impl SimEngine for CudaEngine {
    fn step(
        &mut self,
        agents: &mut AgentState,
        bbo: &BBO,
        tick: u64,
        cancel_agents: &mut Vec<u32>,
        market_orders: &mut Vec<LobOrder>,
        limit_orders: &mut Vec<LobOrder>,
    ) -> (usize, GpuStepTimings) {
        cancel_agents.clear();
        market_orders.clear();
        limit_orders.clear();

        // --- GPU upload (position only, rest is GPU-resident) ---
        let t0 = Instant::now();
        {
            let pos_slice = self.h_pos_f32.as_mut_slice().unwrap();
            for (dst, &src) in pos_slice.iter_mut().zip(agents.position.iter()) {
                *dst = src as f32;
            }
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

        // --- GPU kernel: agent_decide ---
        let t1 = Instant::now();

        // Reset active_count to 0 before compact kernel (async on same stream)
        self.stream.memset_zeros(&mut self.d_active_count).unwrap();

        unsafe {
            self.stream
                .launch_builder(&self.decide_func)
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

        // --- GPU kernel: classify_orders (same stream, no synchronize between) ---
        let participation_threshold = self.participation_threshold;
        let market_order_prob = self.market_order_threshold;
        let tick_size_val = self.tick_size;
        let tick_u32 = tick as u32;
        unsafe {
            self.stream
                .launch_builder(&self.classify_func)
                .arg(&self.d_order_price)
                .arg(&self.d_order_quantity)
                .arg(&self.d_agent_type)
                .arg(&self.d_strategy_params)
                .arg(&self.d_mm_half_spread)
                .arg(&self.d_mm_quote_size)
                .arg(&self.d_mm_requote_threshold)
                .arg(&mut self.d_mm_last_quote_mid)
                .arg(&mut self.d_out_order_type)
                .arg(&mut self.d_out_bid_price)
                .arg(&mut self.d_out_ask_price)
                .arg(&mut self.d_out_qty)
                .arg(&mut self.d_out_cancel_flag)
                .arg(&participation_threshold)
                .arg(&market_order_prob)
                .arg(&tick_size_val)
                .arg(&n_i32)
                .arg(&k_i32)
                .arg(&tick_u32)
                .launch(cfg)
        }
        .unwrap();

        // --- GPU kernel: compact_orders (same stream, no synchronize between) ---
        unsafe {
            self.stream
                .launch_builder(&self.compact_func)
                .arg(&self.d_out_order_type)
                .arg(&self.d_out_bid_price)
                .arg(&self.d_out_ask_price)
                .arg(&self.d_out_qty)
                .arg(&self.d_out_cancel_flag)
                .arg(&mut self.d_compact)
                .arg(&mut self.d_active_count)
                .arg(&n_i32)
                .launch(cfg)
        }
        .unwrap();

        // Synchronize to get accurate kernel execution time (launches are async)
        self.stream.synchronize().unwrap();
        let kernel_time = t1.elapsed();

        // --- GPU download: active count (4 bytes) then compact buffer (M * 24 bytes) ---
        let t2 = Instant::now();

        // 1. Download scalar count and synchronize so we know M.
        self.stream.memcpy_dtoh(&self.d_active_count, &mut self.h_active_count).unwrap();
        self.stream.synchronize().unwrap();
        let active_m = self.h_active_count.as_slice()[0] as usize;

        // 2. Partial download: only active_m entries from d_compact.
        //    Use raw async DtoH with explicit byte count instead of full-N copy.
        if active_m > 0 {
            let (d_ptr, _guard) = self.d_compact.device_ptr(&self.stream);
            let dst_slice = unsafe {
                std::slice::from_raw_parts_mut(self.h_compact.ptr, active_m)
            };
            unsafe {
                cudarc::driver::result::memcpy_dtoh_async(dst_slice, d_ptr, self.stream.cu_stream())
                    .unwrap();
            }
            self.stream.synchronize().unwrap();
        }

        let download_time = t2.elapsed();

        // Sort by agent_id to restore deterministic processing order (atomicAdd
        // in compact kernel is non-deterministic across warps).
        let compact = unsafe { std::slice::from_raw_parts_mut(self.h_compact.ptr, active_m) };
        compact.sort_unstable_by_key(|e| e.agent_id);

        // Build LOB orders from compact entries
        for entry in compact.iter() {
            let otype    = entry.order_type;
            let agent_id = entry.agent_id;

            if entry.cancel_flag != 0 {
                cancel_agents.push(agent_id);
            }

            // MM requote: cancel_flag=1, both bid and ask prices present
            if entry.cancel_flag != 0 && entry.ask_price != 0.0 {
                limit_orders.push(LobOrder {
                    order_id: 0,
                    agent_id,
                    side: Side::Buy,
                    price: entry.bid_price,
                    quantity: entry.qty,
                    order_type: OrderType::Limit,
                    tick: u64::MAX,
                });
                limit_orders.push(LobOrder {
                    order_id: 0,
                    agent_id,
                    side: Side::Sell,
                    price: entry.ask_price,
                    quantity: entry.qty,
                    order_type: OrderType::Limit,
                    tick: u64::MAX,
                });
            } else {
                // Non-MM order: route to market_orders or limit_orders
                let (side, order_type_val) = match otype {
                    1 => (Side::Buy, OrderType::Limit),
                    2 => (Side::Sell, OrderType::Limit),
                    3 => (Side::Buy, OrderType::Market),
                    4 => (Side::Sell, OrderType::Market),
                    _ => continue,
                };
                let order = LobOrder {
                    order_id: 0,
                    agent_id,
                    side,
                    price: entry.bid_price,
                    quantity: entry.qty,
                    order_type: order_type_val,
                    tick,
                };
                match order_type_val {
                    OrderType::Market => market_orders.push(order),
                    OrderType::Limit => limit_orders.push(order),
                }
            }
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
        self.decide_func = func;
        Ok(())
    }
}
