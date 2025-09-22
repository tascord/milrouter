use {
    hyper::HeaderMap,
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

pub use milrouter_macros::*;
pub use {anyhow, tokio};

#[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
pub use {bytes, futures::future::BoxFuture, http_body_util, hyper, hyper_util, serde_json, tracing};

/// Endpoint attribute allowing all requests through, regardless of authentication headers.
///
/// Example:
/// `#[endpoint(auth = all_aboard)]`
pub async fn all_aboard(_: HeaderMap) -> anyhow::Result<()> { Ok(()) }

pub trait Endpoint<C> {
    type Data: DeserializeOwned + Serialize + Send;
    type Returns: DeserializeOwned + Serialize + Send;

    fn is_idempotent() -> bool;
}

pub trait Router: Display + Sized + Send {}

pub trait IntoRouter<R: Router> {
    #[must_use]
    fn router(self) -> R;
}

pub fn gz_compress(mut inp: impl Read, out: &mut impl Write) -> anyhow::Result<()> {
    let mut out = flate2::write::GzEncoder::new(out, flate2::Compression::default());
    std::io::copy(&mut inp, &mut out)?;

    Ok(())
}
