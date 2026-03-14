//! Benchmark utilities for LOD operations

#[cfg(feature = "bench")]
pub use criterion::Criterion;

/// Re-export for benchmark harness
#[cfg(feature = "bench")]
pub fn setup_benchmarks() -> Criterion {
    Criterion::default()
}
