/// Bump arena for query-scoped allocations (avoid hot-path heap churn).
pub struct Arena {
    _buf: Vec<u8>,
}

impl Arena {
    pub fn new(capacity: usize) -> Self {
        Self { _buf: Vec::with_capacity(capacity) }
    }
}
