/// Quantization metadata for compression-native execution.
#[derive(Debug, Clone, Default)]
pub struct QuantConfig {
    pub bits: u8,
    pub dim: u32,
}
