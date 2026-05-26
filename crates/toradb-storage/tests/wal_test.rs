use std::path::Path;

use toradb_storage::wal::{append_flush, read_flushes, sync_flush_log};

#[test]
fn wal_append_and_read_flush_records() {
    let dir = std::env::temp_dir().join("toradb_wal_test");
    let _ = std::fs::remove_dir_all(&dir);
    let base = Path::new(&dir);

    append_flush(base, "papers", "seg_00001.parquet", 0, 3, true).expect("append");
    append_flush(base, "papers", "seg_00002.parquet", 3, 2, true).expect("append");

    let records = read_flushes(base, "papers").expect("read");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].segment, "seg_00001.parquet");
    assert_eq!(records[0].since_id, 0);
    assert_eq!(records[0].doc_count, 3);
    assert_eq!(records[1].doc_count, 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wal_buffered_append_then_sync() {
    let dir = std::env::temp_dir().join("toradb_wal_buffered");
    let _ = std::fs::remove_dir_all(&dir);
    let base = Path::new(&dir);

    append_flush(base, "docs", "seg_00001.parquet", 0, 10, false).expect("append");
    append_flush(base, "docs", "seg_00002.parquet", 10, 5, false).expect("append");
    sync_flush_log(base, "docs").expect("sync");

    let records = read_flushes(base, "docs").expect("read");
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].since_id, 10);

    let _ = std::fs::remove_dir_all(&dir);
}
