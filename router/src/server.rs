use {
    crate::Endpoint,
    futures::future::BoxFuture,
    http_body_util::Full,
    hyper::{
        HeaderMap,
        body::{Bytes, Frame},
    },
    hyper_util::rt::TokioIo,
    std::{
        marker::PhantomData,
        pin::Pin,
        task::{Context, Poll},
    },
    tokio::net::TcpStream,
};

pub struct IOTypeNotSend {
    _marker: PhantomData<*const ()>,
    stream: TokioIo<TcpStream>,
}

impl IOTypeNotSend {
    pub fn new(stream: TokioIo<TcpStream>) -> Self { Self { _marker: PhantomData, stream } }
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
}

impl hyper::body::Body for Body {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(self.get_mut().data.take().map(|d| Ok(Frame::data(d))))
    }
}

pub type AsyncHandler<I, O> = Box<dyn Fn(I) -> BoxFuture<'static, O> + Send + 'static>;

pub type AsyncHandler3<I, I2, I3, O> = Box<dyn Fn(I, I2, I3) -> BoxFuture<'static, O> + Send + 'static>;

#[allow(clippy::type_complexity)]
pub trait ServerEndpoint<C>: Endpoint<C> {
    fn auth() -> AsyncHandler<HeaderMap, Result<C, anyhow::Error>>;
    fn handler() -> AsyncHandler3<C, HeaderMap, <Self as Endpoint<C>>::Data, anyhow::Result<<Self as Endpoint<C>>::Returns>>;
}
