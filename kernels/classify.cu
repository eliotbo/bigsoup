// CUDA kernel: classify raw (price, qty) from agent_decide into LOB order types.
// Runs on the same stream immediately after agent_decide — no synchronize between them.
// The intermediate d_order_price / d_order_quantity stay in GPU HBM.

extern "C" __global__ void classify_orders(
    const float* order_price,
    const float* order_quantity,
    const int*   agent_type,         // 0 = normal, 1 = MM
    const float* strategy_params,
    const float* mm_half_spread,
    const float* mm_quote_size,
    const float* mm_requote_threshold,
    float*       mm_last_quote_mid,  // read + write
    int*   out_order_type,           // 0=skip, 1=limit_buy, 2=limit_sell, 3=market_buy, 4=market_sell
    float* out_bid_price,
    float* out_ask_price,
    float* out_qty,
    int*   out_cancel_flag,          // 1 = cancel resting orders for this agent
    float  participation_threshold,
    float  market_order_prob,        // probability [0,1] a non-MM submits a market order; 0 = always limit
    float  tick_size,
    int    N,
    int    K,
    unsigned int tick
) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= N) return;

    float price   = order_price[i];
    float qty     = order_quantity[i];
    float abs_qty = fabsf(qty);

    // Default: skip
    out_order_type[i]  = 0;
    out_bid_price[i]   = 0.0f;
    out_ask_price[i]   = 0.0f;
    out_qty[i]         = 0.0f;
    out_cancel_flag[i] = 0;

    int   is_mm      = agent_type[i];
    float aggression = strategy_params[i * K + 0];

    if (!is_mm) {
        // ---- Non-MM agent ----

        // Skip near-zero quantities (matches f32::EPSILON)
        if (abs_qty < 1.1920929e-7f) return;

        // Participation filter
        if (participation_threshold > 0.0f &&
            abs_qty * aggression < participation_threshold)
            return;

        int is_buy = qty > 0.0f;

        // Hash agent index and tick to get a stable per-agent-per-tick random float in [0,1).
        unsigned int h = (unsigned int)i ^ (tick * 2654435761u);
        h ^= h >> 16; h *= 0x45d9f3bu; h ^= h >> 16;
        float r = (float)(h >> 16) / 65536.0f;
        int is_market = market_order_prob > 0.0f && r < market_order_prob;

        if (is_market) {
            out_order_type[i] = is_buy ? 3 : 4;
            out_bid_price[i]  = price;
            out_qty[i]        = abs_qty;
        } else {
            // Limit order with tick rounding
            float rounded = price;
            if (tick_size != 0.0f) {
                rounded = roundf(price / tick_size) * tick_size;
            }
            out_order_type[i] = is_buy ? 1 : 2;
            out_bid_price[i]  = rounded;
            out_qty[i]        = abs_qty;
        }
    } else {
        // ---- Market maker ----
        float signal_price   = price;
        float last_mid       = mm_last_quote_mid[i];
        float drift          = fabsf(signal_price - last_mid);
        float requote_thresh = mm_requote_threshold[i];

        if (last_mid == 0.0f || drift > requote_thresh) {
            float half_spread = mm_half_spread[i];
            float quote_size  = mm_quote_size[i];

            float bid = signal_price - half_spread;
            float ask = signal_price + half_spread;
            if (tick_size != 0.0f) {
                bid = roundf(bid / tick_size) * tick_size;
                ask = roundf(ask / tick_size) * tick_size;
            }

            out_cancel_flag[i] = 1;
            out_order_type[i]  = 1;   // MM requote marker (bid in bid_price, ask in ask_price)
            out_bid_price[i]   = bid;
            out_ask_price[i]   = ask;
            out_qty[i]         = quote_size;

            mm_last_quote_mid[i] = signal_price;
        }
        // else: drift small → all outputs stay 0 (skip), keep existing quotes
    }
}
