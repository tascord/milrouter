use {
    server::{DemoRouter, SearchQuery},
    std::time::Duration,
};

#[test]
fn typed_client_roundtrip_covers_query_tools() -> Result<(), Box<dyn std::error::Error>> {
    let probe = std::net::TcpListener::bind("127.0.0.1:0")?;
    let addr = probe.local_addr()?;
    drop(probe);

    std::thread::spawn(move || {
        let _ = milrouter::serve_local(addr, DemoRouter::route);
    });

    let mut headers = hyper::HeaderMap::new();
    headers.insert("x-demo-client", hyper::header::HeaderValue::from_static("integration-test"));
    let client = DemoRouter::client(format!("http://{addr}"), headers);

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;

    let mut server_ready = false;
    let mut last_error = String::new();
    for _ in 0..40 {
        match rt.block_on(client.the_time(())) {
            Ok(_) => {
                server_ready = true;
                break;
            }
            Err(e) => {
                last_error = e.to_string();
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    if !server_ready {
        return Err(format!("Server did not become ready in time: {last_error}").into());
    }

    let result = rt.block_on(client.search(SearchQuery {
        needle: "or".to_string(),
        haystack: vec!["router".to_string(), "planet".to_string(), "orbit".to_string()],
    }))?;

    assert_eq!(result.total, 1);
    assert_eq!(result.matches, vec!["orbit".to_string()]);

    let raw = rt.block_on(client.version_blob(()))?;
    assert_eq!(raw, b"milrouter-demo-v2\n".to_vec());

    Ok(())
}
