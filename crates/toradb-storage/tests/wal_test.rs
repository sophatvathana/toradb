use std::path::Path;

use toradb_storage::wal::{append_flush, read_flushes};

#[test]
fn wal_append_and_read_flush_records() {
    let dir = std::env::temp_dir().join("toradb_wal_test");
    let _ = std::fs::remove_dir_all(&dir);
    let base = Path::new(&dir);

    append_flush(base, "papers", "seg_00001.parquet", 0, 3).expect("append");
    append_flush(base, "papers", "seg_00002.parquet", 3, 2).expect("append");

    let records = read_flushes(base, "papers").expect("read");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].segment, "seg_00001.parquet");
    assert_eq!(records[0].since_id, 0);
    assert_eq!(records[0].doc_count, 3);
    assert_eq!(records[1].doc_count, 2);

    let _ = std::fs::remove_dir_all(&dir);
}
