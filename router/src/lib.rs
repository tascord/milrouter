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

pub trait Endpoint<C> {
    type Data: DeserializeOwned + Serialize + Send;
    /// The return type of the endpoint.
    /// For streaming endpoints this will be [`ResponseStream`]; for raw endpoints [`Vec<u8>`].
    type Returns: Send;

    fn is_idempotent() -> bool;
    /// The URL path segment for this endpoint (snake_case function name).
    fn path() -> &'static str;
}

pub trait Router: Display + Sized + Send {
    fn route(
        &self,
        req: hyper::Request<Incoming>,
    ) -> futures::future::BoxFuture<'static, Result<hyper::Response<MilBody>, std::convert::Infallible>>;

    fn middleware(&self) -> Vec<Box<dyn Middleware>>;
}

pub trait IntoRouter<R: Router> {
    #[must_use]
    fn router(self) -> R;
}

pub fn gz_compress(mut inp: impl Read, out: &mut impl Write) -> anyhow::Result<()> {
    let mut out = flate2::write::GzEncoder::new(out, flate2::Compression::default());
    std::io::copy(&mut inp, &mut out)?;

    Ok(())
}

pub fn gz_decompress(mut inp: impl Read, out: &mut impl Write) -> anyhow::Result<()> {
    let mut decoder = flate2::read::GzDecoder::new(&mut inp);
    std::io::copy(&mut decoder, out)?;

    Ok(())
}

pub trait Middleware: Send {
    fn route(&mut self, req: &Request<Incoming>) -> futures::future::BoxFuture<'static, anyhow::Result<Option<Response<Full<Bytes>>>>>;
}