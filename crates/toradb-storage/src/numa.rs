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

impl NumaConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("TORADB_NUMA_NODE") {
            if let Ok(n) = v.parse() {
                cfg.node = n;
            }
        }
        if let Ok(v) = std::env::var("TORADB_PREFETCH") {
            cfg.prefetch = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
        cfg
    }
}

/// Hint the OS to prefetch mmap-backed bytes sequentially (best-effort).
pub fn prefetch_mmap_sequential(data: &[u8]) {
    if data.is_empty() {
        return;
    }
    #[cfg(unix)]
    {
        extern "C" {
            fn madvise(addr: *mut libc::c_void, len: libc::size_t, advice: libc::c_int) -> libc::c_int;
        }
        const MADV_WILLNEED: libc::c_int = 3;
        unsafe {
            let _ = madvise(data.as_ptr() as *mut _, data.len(), MADV_WILLNEED);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = data;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefetch_no_panic_on_empty() {
        prefetch_mmap_sequential(&[]);
        prefetch_mmap_sequential(&[1, 2, 3]);
    }
}
