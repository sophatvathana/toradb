//! SIMD kernels: distance, decompress, bitmap ops.

pub mod bitmap;
pub mod decompress;
pub mod dispatch;
pub mod distance;
pub mod fht;
mod kernels;
pub mod tq_adc;

pub use dispatch::SimdLevel;
pub use distance::dot_f32;
