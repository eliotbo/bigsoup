// CUDA kernel mirroring cpu_engine.rs logic exactly.
// Param layout (K=8): aggression, mean_reversion, trend_follow, noise_scale,
//                     ema_alpha, fair_value_lr, position_limit, risk_aversion
// Internal state layout (M=4): fair_value_estimate, ema, prev_mid, rng_state

extern "C" __global__ void agent_decide(
    float best_bid,
    float best_ask,
    float last_price,
    float fair_value,
    const float* position,
    const float* cash,
    const float* strategy_params,
    float* internal_state,
    float* order_price,
    float* order_quantity,
    int N,
    int K,
    int M
) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= N) return;

    // --- Load strategy params ---
    float aggression     = strategy_params[i * K + 0];
    float mean_reversion = strategy_params[i * K + 1];
    float trend_follow   = strategy_params[i * K + 2];
    float noise_scale    = strategy_params[i * K + 3];
    float ema_alpha      = strategy_params[i * K + 4];
    float fair_value_lr  = strategy_params[i * K + 5];
    float position_limit = strategy_params[i * K + 6];
    float risk_aversion  = strategy_params[i * K + 7];

    // --- Load internal state ---
    float fair_est = internal_state[i * M + 0];
    float ema      = internal_state[i * M + 1];
    // prev_mid is index 2, not used in computation directly
    unsigned int rng = __float_as_uint(internal_state[i * M + 3]);

    // --- Compute mid and spread ---
    float mid    = (best_bid + best_ask) * 0.5f;
    float spread = best_ask - best_bid;

    // --- Update EMA ---
    ema = ema + ema_alpha * (mid - ema);

    // --- Update fair value estimate ---
    // Update fair value estimate toward exogenous fundamental
    fair_est = fair_est + fair_value_lr * (fair_value - fair_est);

    // --- Signals ---
    float mr_signal = (fair_est - mid) * mean_reversion;
    float tf_signal = (mid - ema) * trend_follow;

    // --- LCG noise ---
    rng = rng * 1664525u + 1013904223u;
    float noise = (float)(rng & 0xFFFF) / 65535.0f - 0.5f;
    noise *= noise_scale * spread;

    // --- Position penalty ---
    float pos = position[i];
    float pos_penalty = -risk_aversion * pos;

    // --- Combined signal ---
    float signal = mr_signal + tf_signal + noise + pos_penalty;

    // --- Order price and quantity ---
    float order_px  = mid + signal * aggression;
    float order_qty = signal;

    // --- Clamp quantity by position limits ---
    if (pos + order_qty > position_limit) {
        order_qty = position_limit - pos;
    }
    if (pos + order_qty < -position_limit) {
        order_qty = -position_limit - pos;
    }

    // --- Write outputs ---
    order_price[i]    = order_px;
    order_quantity[i] = order_qty;

    // --- Write back internal state ---
    internal_state[i * M + 0] = fair_est;
    internal_state[i * M + 1] = ema;
    internal_state[i * M + 2] = mid;
    internal_state[i * M + 3] = __uint_as_float(rng);
}
