#[derive(Debug, Clone, Copy)]
pub struct NumaConfig {
    pub node: u32,
    pub prefetch: bool,
}

impl Default for NumaConfig {
    fn default() -> Self {
        Self { node: 0, prefetch: true }
    }
}
