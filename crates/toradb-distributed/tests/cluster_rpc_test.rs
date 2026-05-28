use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use toradb_distributed::config::{ClusterConfig, WorkerNode};
use toradb_distributed::protocol::{Request, Response};
use toradb_distributed::rpc;
use toradb_distributed::{ClusterClient, Coordinator, Worker};

#[test]
fn cluster_health_and_segment_rpc() {
    let dir = std::env::temp_dir().join("toradb_cluster_rpc");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let addr = "127.0.0.1:19107";
    let dir_c = dir.clone();
    let handle = thread::spawn(move || {
        let worker = Worker::new(dir_c);
        let listener = TcpListener::bind(addr).expect("bind");
        for _ in 0..8 {
            if let Ok((mut stream, _)) = listener.accept() {
                let req = match rpc::recv_request(&mut stream) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = rpc::send_response(&mut stream, &Response::err(e));
                        continue;
                    }
                };
                let resp = worker.handle(req);
                let _ = rpc::send_response(&mut stream, &resp);
            }
        }
    });
    thread::sleep(Duration::from_millis(80));

    let ok = matches!(rpc::call(addr, &Request::Health), Ok(Response::Ok { .. }));
    assert!(ok, "health rpc");

    let config = ClusterConfig {
        coordinator_db: dir.clone(),
        workers: vec![WorkerNode {
            id: "w0".into(),
            addr: addr.into(),
            db_path: dir.clone(),
        }],
    };
    let client = ClusterClient::new(config.clone());
    let status = client.health_all().expect("health");
    assert!(status.iter().any(|(_, ok)| *ok));

    let coord = Coordinator::new(&config);
    let merged = coord
        .search_segments("docs", "motor", 5, &[0])
        .unwrap_or_default();
    assert!(merged.is_empty());

    drop(handle);
    let _ = std::fs::remove_dir_all(&dir);
}
