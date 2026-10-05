// Install a later-dispatch fault only after a real closed Mapper has returned.
// SQLite authorizes trigger bodies even when their WHEN is false; placing these
// before bootstrap now legitimately exercises the earlier admission boundary.
struct MapperBoundaryEndpoint {
    port: u16,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    database: std::sync::Arc<std::sync::Mutex<Option<PathBuf>>>,
}
fn coordinator_mapper_boundary_endpoint(sql: &'static str) -> MapperBoundaryEndpoint {
    let database = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let path = database.clone();
    let (port, requests, _) = spawn_endpoint(std::sync::Arc::new(move |request| {
        if request.contains("You are the Root Coordinator") && request.contains("mapper-output") {
            let db = db::open(path.lock().unwrap().as_ref().unwrap()).unwrap();
            db.execute_batch(sql).unwrap();
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    MapperBoundaryEndpoint {
        port,
        requests,
        database,
    }
}
