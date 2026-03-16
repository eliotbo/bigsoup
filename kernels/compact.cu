// CUDA kernel: stream-compact classify output into a dense AoS buffer.
// Reads the 5 classified output arrays produced by classify_orders and packs
// only active entries (order_type != 0 OR cancel_flag != 0) into compact_out,
// writing the active count to active_count (must be zeroed before launch).
//
// Uses atomicAdd for write-index assignment.  The output order is
// non-deterministic across warps; the Rust caller sorts by agent_id.

struct CompactOrder {
    unsigned int agent_id;
    int          order_type;
    float        bid_price;
    float        ask_price;
    float        qty;
    int          cancel_flag;
};  // 24 bytes

extern "C" __global__ void compact_orders(
    const int*    order_type,
    const float*  bid_price,
    const float*  ask_price,
    const float*  qty,
    const int*    cancel_flag,
    CompactOrder* compact_out,
    unsigned int* active_count,
    int N
) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= N) return;

    int otype = order_type[i];
    int cflag  = cancel_flag[i];
    if (otype == 0 && cflag == 0) return;

    unsigned int idx = atomicAdd(active_count, 1u);
    compact_out[idx].agent_id   = (unsigned int)i;
    compact_out[idx].order_type = otype;
    compact_out[idx].bid_price  = bid_price[i];
    compact_out[idx].ask_price  = ask_price[i];
    compact_out[idx].qty        = qty[i];
    compact_out[idx].cancel_flag = cflag;
}
