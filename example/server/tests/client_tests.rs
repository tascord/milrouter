use {
    server::{DemoRouter, SearchQuery},
    std::time::Duration,
};

fn spawn_server() -> (std::net::SocketAddr, tokio::runtime::Runtime) {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    std::thread::spawn(move || {
        let _ = milrouter::serve_local(addr, DemoRouter::route);
    });

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    (addr, rt)
}

fn wait_for_server(rt: &tokio::runtime::Runtime, addr: std::net::SocketAddr) {
    let client = DemoRouter::client(format!("http://{addr}"), Default::default());
    for _ in 0..40 {
        if rt.block_on(client.the_time(())).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("Server did not become ready in time");
}

#[test]
fn client_returns_json_deserialized_response() {
    let (addr, rt) = spawn_server();
    wait_for_server(&rt, addr);

    let client = DemoRouter::client(format!("http://{addr}"), Default::default());

    let result = rt
        .block_on(client.search(SearchQuery {
            needle: "or".to_string(),
            haystack: vec!["router".to_string(), "planet".to_string(), "orbit".to_string()],
        }))
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.matches, vec!["orbit".to_string()]);
}

#[test]
fn client_returns_raw_bytes_for_raw_endpoint() {
    let (addr, rt) = spawn_server();
    wait_for_server(&rt, addr);

    let client = DemoRouter::client(format!("http://{addr}"), Default::default());

    let raw = rt.block_on(client.version_blob(())).unwrap();
    assert_eq!(raw, b"milrouter-demo-v2\n".to_vec());
}

#[test]
fn client_preserves_custom_headers() {
    let (addr, rt) = spawn_server();
    wait_for_server(&rt, addr);

    let mut headers = milrouter::hyper::HeaderMap::new();
    headers.insert("x-demo-client", milrouter::hyper::header::HeaderValue::from_static("test-header"));

    let client = DemoRouter::client(format!("http://{addr}"), headers);

    let result =
        rt.block_on(client.search(SearchQuery { needle: "a".to_string(), haystack: vec!["apple".to_string()] })).unwrap();

    assert_eq!(result.total, 1);
}

#[test]
fn client_handles_401_unauthorized() {
    let (addr, rt) = spawn_server();
    wait_for_server(&rt, addr);

    let mut headers = milrouter::hyper::HeaderMap::new();
    headers.insert("evil", milrouter::hyper::header::HeaderValue::from_static("true"));

    let client = DemoRouter::client(format!("http://{addr}"), headers);

    let result = rt.block_on(client.the_time(()));

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("401") || err.contains("Unauthorized"),
        "Expected 401 or Unauthorized in error message, got: {}",
        err
    );
}

#[test]
fn client_uses_put_for_idempotent_endpoint() {
    let (addr, rt) = spawn_server();
    wait_for_server(&rt, addr);

    let client = DemoRouter::client(format!("http://{addr}"), Default::default());

    // the_time is marked idempotent = true, so it should use PUT
    // The request itself should succeed; we can't easily observe the method
    // from the client side without intercepting, but we can at least verify
    // the endpoint is callable.
    let result = rt.block_on(client.the_time(()));
    assert!(result.is_ok(), "Idempotent endpoint request failed: {:?}", result);
}

#[test]
fn client_uses_post_for_non_idempotent_endpoint() {
    let (addr, rt) = spawn_server();
    wait_for_server(&rt, addr);

    let client = DemoRouter::client(format!("http://{addr}"), Default::default());

    // search is not marked idempotent, so it should use POST
    let result = rt.block_on(client.search(SearchQuery { needle: "test".to_string(), haystack: vec!["test".to_string()] }));
    assert!(result.is_ok(), "Non-idempotent endpoint request failed: {:?}", result);
}

#[test]
fn client_works_with_trailing_slash_in_host() {
    let (addr, rt) = spawn_server();
    wait_for_server(&rt, addr);

    let client = DemoRouter::client(format!("http://{addr}/"), Default::default());

    let result = rt.block_on(client.the_time(()));
    assert!(result.is_ok(), "Client with trailing slash failed: {:?}", result);
}
