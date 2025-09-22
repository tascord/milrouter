use {
    crate::{Endpoint, IntoRouter, Router},
    anyhow::anyhow,
    futures::{FutureExt, future::BoxFuture},
    futures_signals::signal::Mutable,
    std::sync::Arc,
    tokio::sync::oneshot::Receiver,
    wasm_bindgen_futures::spawn_local,
};

pub struct WasmReponse<T>(Receiver<anyhow::Result<T>>);
impl<T> WasmReponse<T> {
    pub fn new(v: Receiver<anyhow::Result<T>>) -> Self { Self(v) }
}

impl<T> IntoFuture for WasmReponse<T>
where
    T: Send + 'static,
{
    type IntoFuture = BoxFuture<'static, Self::Output>;
    type Output = anyhow::Result<T>;

    fn into_future(self) -> Self::IntoFuture {
        let fut = IntoFuture::into_future(self.0).map(|v| match v {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(anyhow!(e.to_string())),
        });

        Box::pin(fut)
    }
}

impl<R: Send + 'static> WasmReponse<R> {
    #[must_use]
    pub fn as_mutable(self) -> Mutable<Fetch<Arc<R>, Arc<anyhow::Error>>> {
        let m = Mutable::new(Fetch::Loading);
        self.with_mutable(&m);
        m
    }

    pub fn with_mutable(self, m: &Mutable<Fetch<Arc<R>, Arc<anyhow::Error>>>) {
        let m = m.clone();

        spawn_local({
            async move {
                m.set(match self.await {
                    Ok(t) => Fetch::Finished(Arc::new(t)),
                    Err(e) => Fetch::Error(Arc::new(e)),
                });
            }
        });
    }
}

/// This is one way to make requests.
/// You may (and probably should) customise this to fir your needs.
pub fn request<R, C, E>(endpoint: E, data: E::Data) -> WasmReponse<E::Returns>
where
    E: Endpoint<C> + IntoRouter<R> + Send + 'static,
    E::Data: Send + 'static,
    E::Returns: 'static,
    R: Router,
{
    let base_url = web_sys::window().unwrap().origin();
    let (tx, rx) = tokio::sync::oneshot::channel::<anyhow::Result<E::Returns>>();

    spawn_local(async move {
        let _ = tx.send(
            async move {
                let req = reqwest::Client::new()
                    .request(
                        match E::is_idempotent() {
                            true => reqwest::Method::PUT,
                            false => reqwest::Method::POST,
                        },
                        format!("{base_url}/{}", endpoint.router()),
                    )
                    .header("Connection", "Keep-Alive")
                    .header("Keep-Alive", "timeout=600")
                    .json(&data)
                    .send()
                    .await?;

                let resp = req.text().await?;

                Ok(serde_json::from_str(&resp)
                    .inspect_err(|e| tracing::error!("Request error: {:#?}, body: {:#?}", e, resp))?)
            }
            .await,
        );
    });

    WasmReponse(rx)
}

#[derive(Debug, Default, Clone)]
pub enum Fetch<T: Clone, E: Clone> {
    #[default]
    Waiting,
    Loading,
    Finished(T),
    Error(E),
}
// this is intentional, its just to compare state not the underlying data or errors
impl<T: Clone, E: Clone> PartialEq for Fetch<T, E> {
    fn eq(&self, other: &Self) -> bool { core::mem::discriminant(self) == core::mem::discriminant(other) }
}
impl<T: Clone, E: Clone + ToString> Eq for Fetch<T, E> {}

impl<T: Clone, E: Clone + ToString> Fetch<T, E> {
    pub fn unwrap_or_default(self) -> T
    where
        T: Default,
    {
        match self {
            Fetch::Waiting => T::default(),
            Fetch::Loading => T::default(),
            Fetch::Error(_) => T::default(),
            Fetch::Finished(v) => v,
        }
    }

    #[must_use]
    pub fn as_opt(self) -> Option<T> {
        match self {
            Fetch::Waiting => None,
            Fetch::Loading => None,
            Fetch::Finished(v) => Some(v),
            Fetch::Error(_) => None,
        }
    }

    pub fn result(self) -> anyhow::Result<T> {
        match self {
            Fetch::Waiting => Err(anyhow::anyhow!("Request not started.")),
            Fetch::Loading => Err(anyhow::anyhow!("Request in progress.")),
            Fetch::Finished(v) => Ok(v),
            Fetch::Error(e) => Err(anyhow::anyhow!(e.to_string())),
        }
    }
}
