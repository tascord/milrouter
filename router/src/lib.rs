use {
    hyper::{HeaderMap, body::Incoming},
    serde::{Serialize, de::DeserializeOwned},
    std::{
        fmt::Display,
        io::{Read, Write},
    },
};

#[cfg(any(target_arch = "wasm32", target_arch = "wasm64"))]
pub mod wasm;

#[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
pub mod server;

#[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
pub use server::*;
pub use {anyhow, milrouter_macros::*, tokio};
#[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
pub use {bytes, futures, futures::future::BoxFuture, http_body_util, hyper, hyper_util, reqwest, serde, serde_json, tracing};
use {
    bytes::Bytes,
    http_body_util::Full,
    hyper::{Request, Response},
};

/// Endpoint attribute allowing all requests through, regardless of authentication headers.
///
/// Example:
/// `#[endpoint(auth = all_aboard)]`
pub async fn all_aboard(_: HeaderMap) -> anyhow::Result<()> { Ok(()) }

/// Implemented automatically by `#[endpoint]` for every endpoint function.
///
/// You normally never implement this by hand.
pub trait Endpoint<C> {
    /// The request/argument type for this endpoint.
    type Data: DeserializeOwned + Serialize + Send;
    /// The return type of the endpoint.
    /// For streaming endpoints this will be [`ResponseStream`]; for raw endpoints [`Vec<u8>`].
    type Returns: Send;

    /// `true` for endpoints declared with `idempotent = true` (uses `PUT`).
    fn is_idempotent() -> bool;
    /// The URL path segment for this endpoint (snake_case function name).
    fn path() -> &'static str;
}

/// Implemented automatically by `#[derive(Router)]` for router enums.
///
/// `#[derive(Router)]` also generates:
/// - A `new()` constructor.
/// - A `route()` associated function used by the server helpers.
/// - A typed `client()` on non-wasm targets.
pub trait Router: Display + Sized + Send {
    /// Route an incoming request and return the response.
    fn route(
        &self,
        req: hyper::Request<Incoming>,
    ) -> futures::future::BoxFuture<'static, Result<hyper::Response<MilBody>, std::convert::Infallible>>;

    /// Return the list of middlewares attached to this router.
    fn middleware(&self) -> Vec<Box<dyn Middleware>>;
}

/// Bridges an endpoint struct back into the router enum that owns it.
/// Implemented automatically by `#[derive(Router)]`.
pub trait IntoRouter<R: Router> {
    #[must_use]
    fn router(self) -> R;
}

/// Gzip-compress `inp` into `out`. Used internally for JSON payloads.
pub fn gz_compress(mut inp: impl Read, out: &mut impl Write) -> anyhow::Result<()> {
    let mut out = flate2::write::GzEncoder::new(out, flate2::Compression::default());
    std::io::copy(&mut inp, &mut out)?;

    Ok(())
}

/// Gzip-decompress `inp` into `out`. Used internally by the typed client.
pub fn gz_decompress(mut inp: impl Read, out: &mut impl Write) -> anyhow::Result<()> {
    let mut decoder = flate2::read::GzDecoder::new(&mut inp);
    std::io::copy(&mut decoder, out)?;

    Ok(())
}

/// Middleware that can intercept requests before they reach an endpoint
/// and transform responses after the endpoint has run.
///
/// Both methods have default no-op implementations, so you only need to
/// override the hook(s) you care about.
///
/// # Example
///
/// ```ignore
/// struct CorsMiddleware {
///     origin: Option<String>,
/// }
///
/// impl CorsMiddleware {
///     fn new() -> Self { CorsMiddleware { origin: None } }
/// }
///
/// impl Middleware for CorsMiddleware {
///     fn before(
///         &mut self,
///         req: &Request<Incoming>,
///     ) -> BoxFuture<'static, anyhow::Result<Option<Response<Full<Bytes>>>>> {
///         self.origin = req.headers()
///             .get("origin")
///             .and_then(|v| v.to_str().ok())
///             .map(|s| s.to_string());
///
///         if req.method() == hyper::Method::OPTIONS {
///             let res = Response::builder()
///                 .status(200)
///                 .header("Access-Control-Allow-Origin", "*")
///                 .body(Full::new(Bytes::new()))
///                 .unwrap();
///             Box::pin(async { Ok(Some(res)) })
///         } else {
///             Box::pin(async { Ok(None) })
///         }
///     }
///
///     fn after(
///         &mut self,
///         res: &mut Response<()>,
///     ) -> BoxFuture<'static, anyhow::Result<()>> {
///         res.headers_mut().insert(
///             "Access-Control-Allow-Origin",
///             hyper::header::HeaderValue::from_static("*"),
///         );
///         Box::pin(async { Ok(()) })
///     }
/// }
/// ```
pub trait Middleware: Send {
    /// Inspect or reject the incoming request before it reaches the endpoint.
    ///
    /// Return `Ok(Some(response))` to short-circuit the router (e.g. rate
    /// limiting, CORS preflight).  Return `Ok(None)` to let the request
    /// continue to the next middleware or the endpoint.
    fn before(
        &mut self,
        _req: &Request<Incoming>,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<Option<Response<Full<Bytes>>>>> {
        Box::pin(async { Ok(None) })
    }

    /// Inspect or mutate the response *after* the endpoint (or another
    /// middleware) has produced it.
    ///
    /// The body is hidden (`Response<()>`) so you cannot accidentally consume
    /// it.  You can still read or modify headers and the status code.  Any
    /// changes are merged back into the real response before it is sent.
    ///
    /// If you need request data here, clone it into `self` during [`before`].
    fn after(
        &mut self,
        _res: &mut Response<()>,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}