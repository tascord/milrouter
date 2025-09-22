use {
    hyper::{server::conn::http1, service::service_fn},
    hyper_util::rt::TokioIo,
    milrouter::{
        server::IOTypeNotSend,
        tracing::{Level, info, warn},
    },
    server::DemoRouter,
    std::env,
    tokio::net::TcpListener,
};

pub fn serve<RouteFut>(
    route: fn(hyper::Request<hyper::body::Incoming>) -> RouteFut,
) -> std::result::Result<(), Box<dyn std::error::Error>>
where
    RouteFut: Future<Output = std::result::Result<hyper::Response<http_body_util::Full<bytes::Bytes>>, std::convert::Infallible>>
        + 'static,
{
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let ls = tokio::task::LocalSet::new();

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", env::var("PORT").unwrap_or("40000".to_string())).parse()?;
    let listener = ls.block_on(&rt, TcpListener::bind(addr))?;

    info!("Listening on http://{}", addr);

    loop {
        let (stream, _) = ls.block_on(&rt, listener.accept())?;
        let io = IOTypeNotSend::new(TokioIo::new(stream));

        let service = service_fn(route);
        ls.spawn_local(async move {
            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                warn!("Error serving connection: {:?}", err);
            }
        });
    }
}

fn main() {
    // Setup logging
    tracing_subscriber::fmt().with_max_level(Level::TRACE).init();

    // Host
    serve(DemoRouter::route).unwrap()
}
