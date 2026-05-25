#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CompressionConfig {
    pub enabled: bool,
    pub block_size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IndexMode {
    Text,
    Hybrid,
    Vector,
}
