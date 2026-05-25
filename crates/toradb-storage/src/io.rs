use std::path::Path;

/// Read an entire file (sync fallback used on all platforms).
pub fn read_file_sync(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| e.to_string())
}

#[cfg(all(feature = "io-uring", target_os = "linux"))]
pub fn read_file_io_uring(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::unix::io::AsRawFd;

    use io_uring::{opcode, types, IoUring};

    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len() as usize;
    if len == 0 {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u8; len];
    let ring = IoUring::new(8).map_err(|e| e.to_string())?;
    let read_e = opcode::Read::new(
        types::Fd(file.as_raw_fd()),
        buf.as_mut_ptr(),
        len as u32,
    )
    .build()
    .user_data(0x42);
    unsafe {
        ring.submission()
            .push(&read_e)
            .map_err(|e| e.to_string())?;
    }
    ring.submit_and_wait(1).map_err(|e| e.to_string())?;
    let cqe = ring
        .completion()
        .next()
        .ok_or("io_uring: missing completion")?;
    if cqe.result() < 0 {
        return Err(format!("io_uring read failed: {}", cqe.result()));
    }
    Ok(buf)
}

#[cfg(not(all(feature = "io-uring", target_os = "linux")))]
pub fn read_file_io_uring(path: &Path) -> Result<Vec<u8>, String> {
    read_file_sync(path)
}
