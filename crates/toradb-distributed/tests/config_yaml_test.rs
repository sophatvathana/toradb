use toradb_distributed::ClusterConfig;

#[test]
fn loads_cluster_yaml() {
    let yaml = r#"
coordinator_db: /tmp/coord
workers:
  - id: w0
    addr: 127.0.0.1:9100
    db_path: /tmp/node0
"#;
    let dir = std::env::temp_dir().join("toradb_cluster_yaml");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cluster.yaml");
    std::fs::write(&path, yaml).unwrap();
    let cfg = ClusterConfig::load(&path).expect("load yaml");
    assert_eq!(cfg.workers.len(), 1);
    assert_eq!(cfg.workers[0].id, "w0");
    let _ = std::fs::remove_dir_all(&dir);
}
