use {
    crate::{Endpoint, Router},
    futures::{Stream, StreamExt, future::BoxFuture},
    http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody},
    hyper::{
        HeaderMap,
        body::{Bytes, Frame, Incoming},
    },
    hyper_util::rt::TokioIo,
    std::{
        future::Future,
        marker::PhantomData,
        net::SocketAddr,
        pin::Pin,
        task::{Context, Poll},
    },
    tokio::net::TcpListener,
};

// ── types ──────────────────────────────────────────────────────────────────

/// A boxed stream of raw byte chunks used by `#[endpoint(stream)]` endpoints.
///
/// Use [`into_response_stream`] to box any concrete stream into this type.
pub type ResponseStream = Pin<Box<dyn Stream<Item = Bytes> + Send + Sync + 'static>>;

/// Box any `Send + Sync` stream of bytes into a [`ResponseStream`].
pub fn into_response_stream(stream: impl Stream<Item = Bytes> + Send + Sync + 'static) -> ResponseStream {
    Box::pin(stream)
}

/// The unified response body type produced by all router `route()` functions.
pub type MilBody = BoxBody<Bytes, std::convert::Infallible>;

// ── IOTypeNotSend (internal hyper helper) ──────────────────────────────────

pub struct IOTypeNotSend {
    _marker: PhantomData<*const ()>,
    stream: TokioIo<tokio::net::TcpStream>,
}

impl IOTypeNotSend {
    pub fn new(stream: TokioIo<tokio::net::TcpStream>) -> Self { Self { _marker: PhantomData, stream } }
}

impl hyper::rt::Write for IOTypeNotSend {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

impl hyper::rt::Read for IOTypeNotSend {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

// ── Body (simple single-chunk body) ───────────────────────────────────────

#[derive(Default)]
pub struct Body {
    _marker: PhantomData<*const ()>,
    data: Option<Bytes>,
}

impl From<String> for Body {
    fn from(value: String) -> Self { Body { _marker: PhantomData, data: Some(value.into()) } }
}

impl<'a> From<&'a [u8]> for Body {
    fn from(value: &'a [u8]) -> Self { Body { _marker: PhantomData, data: Some(Bytes::from_iter(value.iter().cloned())) } }
}

impl Body {
    pub fn full(self) -> Full<Bytes> { Full::new(self.data.unwrap_or_default()) }

    pub fn boxed(self) -> MilBody { self.full().map_err(|never| match never {}).boxed() }
}

impl hyper::body::Body for Body {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(self.get_mut().data.take().map(|d| Ok(Frame::data(d))))
    }
}

// ── handler type aliases ───────────────────────────────────────────────────

pub type AsyncHandler<I, O> = Box<dyn Fn(I) -> BoxFuture<'static, O> + Send + 'static>;

pub type AsyncHandler3<I, I2, I3, O> = Box<dyn Fn(I, I2, I3) -> BoxFuture<'static, O> + Send + 'static>;

// ── ServerEndpoint ─────────────────────────────────────────────────────────

#[allow(clippy::type_complexity)]
pub trait ServerEndpoint<C>: Endpoint<C> {
    fn auth() -> AsyncHandler<HeaderMap, Result<C, anyhow::Error>>;

    /// Handler for normal (JSON) and raw endpoints.
    /// For streaming endpoints use [`stream_handler`] instead.
    fn handler() -> AsyncHandler3<C, HeaderMap, <Self as Endpoint<C>>::Data, anyhow::Result<<Self as Endpoint<C>>::Returns>>;

    /// Returns `true` for `#[endpoint(raw)]` endpoints whose response bytes
    /// are returned as-is rather than JSON-serialised.
    fn is_raw() -> bool { false }

    /// Returns the streaming handler for `#[endpoint(stream)]` endpoints,
    /// or `None` for non-streaming endpoints.
    fn stream_handler() -> Option<AsyncHandler3<C, HeaderMap, <Self as Endpoint<C>>::Data, anyhow::Result<ResponseStream>>> {
        None
    }
}

// ── TypedEndpoint: provides the Client type for router-side codegen ────────

/// Implemented automatically by `#[endpoint]` on non-wasm targets.
/// Lets the router macro refer to the concrete client type without `_`.
pub trait TypedEndpoint: Endpoint<Self::Client> {
    type Client: Send;
}

// ── ClientEndpoint ─────────────────────────────────────────────────────────

/// Implemented by non-streaming endpoint structs to support the typed `.client()` API.
/// The macro generates this impl automatically.
pub trait ClientEndpoint<C>: Endpoint<C> {
    /// Decode the raw HTTP response bytes into `Self::Returns`.
    fn decode_response(bytes: Bytes) -> anyhow::Result<<Self as Endpoint<C>>::Returns>;
}

// ── serve functions ────────────────────────────────────────────────────────

/// Start an HTTP/1 server on a **new single-thread** Tokio runtime using a `LocalSet`.
///
/// This is the simplest way to host a router when you don't already have a
/// Tokio runtime running.  Call from `main()` or any non-async context.
///
/// ```ignore
/// milrouter::serve_local("127.0.0.1:8080".parse().unwrap(), MyRouter).unwrap();
/// ```
pub fn serve_local<R>(addr: SocketAddr, router: R) -> anyhow::Result<()>
where
    R: Router + Send + Sync + 'static,
{
    use {
        hyper::server::conn::http1,
        hyper::service::service_fn,
    };

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let ls = tokio::task::LocalSet::new();

    let listener = ls.block_on(&rt, TcpListener::bind(addr))?;
    tracing::info!("Listening on http://{}", addr);

    let router = std::sync::Arc::new(router);

    loop {
        let (stream, _) = ls.block_on(&rt, listener.accept())?;
        let io = IOTypeNotSend::new(TokioIo::new(stream));
        let router = router.clone();
        let service = service_fn(move |req| router.route(req));
        ls.spawn_local(async move {
            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                tracing::warn!("Error serving connection: {:?}", err);
            }
        });
    }
}

/// Start an HTTP/1 server **inside an existing Tokio runtime**.
///
/// Await this from an `async` context (e.g. inside `#[tokio::main]`).  Each
/// accepted connection is spawned as a normal `tokio::task`.
///
/// ```ignore
/// #[tokio::main]
/// async fn main() {
///     milrouter::serve("127.0.0.1:8080".parse().unwrap(), MyRouter).await.unwrap();
/// }
/// ```
pub async fn serve<R>(addr: SocketAddr, router: R) -> anyhow::Result<()>
where
    R: Router + Send + Sync + 'static,
{
    use {
        hyper::server::conn::http1,
        hyper::service::service_fn,
    };

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Listening on http://{}", addr);

    let router = std::sync::Arc::new(router);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let router = router.clone();
        let service = service_fn(move |req| router.route(req));
        tokio::spawn(async move {
            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                tracing::warn!("Error serving connection: {:?}", err);
            }
        });
    }
}

// ── helper: build a streaming MilBody from a ResponseStream ───────────────

pub fn stream_to_body(stream: ResponseStream) -> MilBody {
    BodyExt::boxed(StreamBody::new(stream.map(|chunk| Ok(Frame::data(chunk)))))
}

// ── helper: re-export fmt for generated Display impls ─────────────────────
pub use std::fmt as _fmt;
