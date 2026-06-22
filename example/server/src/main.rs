use {
    milrouter::tracing::{Level, info},
    server::DemoRouter,
    std::env,
};

fn main() {
    tracing_subscriber::fmt().with_max_level(Level::TRACE).init();

    let addr: std::net::SocketAddr =
        format!("127.0.0.1:{}", env::var("PORT").unwrap_or("40000".to_string()))
            .parse()
            .unwrap();

    info!("Starting milrouter demo on http://{addr}");
    milrouter::serve_local(addr, DemoRouter::route).unwrap();
}
