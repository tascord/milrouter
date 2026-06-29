#![allow(unused)]
use std::collections::HashMap;
use std::ops::Not;
use std::sync::Arc;
use std::time::{Duration, Instant};

// Just dont want to gate even MORE things behind cfgs
use milrouter::{Endpoint, Router, anyhow, endpoint};
use bytes::Bytes;
use hyper::{Request, Response, body::Incoming};
use http_body_util::Full;
use milrouter::futures::future::BoxFuture;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchQuery {
    pub needle: String,
    pub haystack: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub matches: Vec<String>,
    pub total: usize,
}

fn super_awesome_html_generator() -> String {
    "<!doctype html><html><head><meta charset=\"utf-8\"><title>milrouter demo</title></head><body><h1>milrouter demo</h1></body></html>".to_string()
}

pub async fn auth_handler(headers: hyper::HeaderMap) -> anyhow::Result<()> {
    headers.contains_key("evil").not().then_some(()).ok_or(anyhow::anyhow!("Evil request detected."))
}

struct RateLimiter {
    requests: HashMap<String, Vec<Instant>>,
    limit: usize,
    window: Duration,
}

impl RateLimiter {
    fn new(limit: usize, window: Duration) -> Self {
        Self { requests: HashMap::new(), limit, window }
    }

    fn check(&mut self, key: &str) -> bool {
        let now = Instant::now();
        let entries = self.requests.entry(key.to_string()).or_default();
        entries.retain(|&t| now.duration_since(t) < self.window);
        if entries.len() >= self.limit {
            return false;
        }
        entries.push(now);
        true
    }
}

/// Stateful middleware that limits requests per client IP.
struct RateLimitMiddleware {
    limiter: RateLimiter,
}

impl RateLimitMiddleware {
    fn new() -> Self {
        Self { limiter: RateLimiter::new(10, Duration::from_secs(60)) }
    }
}

impl milrouter::Middleware for RateLimitMiddleware {
    /// Short-circuit with 429 when the client exceeds the rate limit.
    fn before(
        &mut self,
        req: &Request<Incoming>,
    ) -> BoxFuture<'static, anyhow::Result<Option<Response<Full<Bytes>>>>> {
        let client_ip = req.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .split(',')
            .next()
            .unwrap_or("unknown")
            .trim()
            .to_string();

        let allowed = self.limiter.check(&client_ip);

        Box::pin(async move {
            if !allowed {
                let res = Response::builder()
                    .status(429)
                    .header("Content-Type", "text/plain")
                    .header("Retry-After", "60")
                    .body(Full::new(Bytes::from("Rate limit exceeded. Try again in 60 seconds.")))
                    .unwrap();
                return Ok(Some(res));
            }
            Ok(None)
        })
    }
}

/// Pipe-style middleware: lets requests through but injects CORS headers into
/// every response via the `after` hook.  Also handles OPTIONS preflight in the
/// `before` hook.
struct CorsMiddleware {
    origin: Option<String>,
}

impl CorsMiddleware {
    fn new() -> Self { CorsMiddleware { origin: None } }
}

impl milrouter::Middleware for CorsMiddleware {
    /// Handle CORS preflight by short-circuiting with the appropriate headers.
    /// Also stash the requested `Origin` so `after` can echo it back.
    fn before(
        &mut self,
        req: &Request<Incoming>,
    ) -> BoxFuture<'static, anyhow::Result<Option<Response<Full<Bytes>>>>> {
        self.origin = req.headers()
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        if req.method() == hyper::Method::OPTIONS {
            let origin = self.origin.as_deref().unwrap_or("*").to_string();
            Box::pin(async move {
                let res = Response::builder()
                    .status(200)
                    .header("Access-Control-Allow-Origin", origin.as_str())
                    .header("Access-Control-Allow-Methods", "GET, POST, PUT, OPTIONS")
                    .header("Access-Control-Allow-Headers", "Content-Type, Authorization")
                    .header("Access-Control-Max-Age", "86400")
                    .body(Full::new(Bytes::new()))
                    .unwrap();
                Ok(Some(res))
            })
        } else {
            Box::pin(async { Ok(None) })
        }
    }

    /// Add CORS headers to every response that makes it past `before`.
    fn after(
        &mut self,
        res: &mut Response<()>,
    ) -> BoxFuture<'static, anyhow::Result<()>> {
        let origin = self.origin.as_deref().unwrap_or("*").to_string();
        res.headers_mut().insert(
            "Access-Control-Allow-Origin",
            hyper::header::HeaderValue::from_str(&origin).unwrap_or_else(|_| hyper::header::HeaderValue::from_static("*")),
        );
        res.headers_mut().insert(
            "Access-Control-Allow-Methods",
            hyper::header::HeaderValue::from_static("GET, POST, PUT, OPTIONS"),
        );
        Box::pin(async move { Ok(()) })
    }
}

#[endpoint(
    idempotent = true,     // Optional. `true` => PUT, `false` (default) => POST.

    auth = auth_handler    // Required.
                           // Function to determine which request are allowed through based on headers.
                           // The inner type returned (e.g unit in this case) is
                           //  passed to the `client` variable of the endpoint, if it exists
                           // We export `all_abord` for an all-in-all-out unit client handler.

)]
fn the_time(
    client: (), // Optional, As above.
    param1: ()  // Optional.
                // If not present, type defaults to unit.

) ->
    anyhow::Result<String> // Must be an anyhow::Result<T>.
                           // Errors given here are given to the client up as 400's.
                           // Panics are given to the client as 500's.
{
    use std::time::{SystemTime, UNIX_EPOCH};
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().to_string())

    // All serialization (requests + responses) are done via serde_json for ease of debugging.
    // If you would like more options (e.g bincoding), create an issue.
}

#[endpoint(auth = auth_handler)]
fn search(headers: hyper::HeaderMap, query: SearchQuery) -> anyhow::Result<SearchResult> {
    let matches = query
        .haystack
        .iter()
        .filter(|candidate| candidate.to_lowercase().contains(&query.needle.to_lowercase()))
        .cloned()
        .collect::<Vec<_>>();

    // Endpoint handlers can read request headers as a normal function argument.
    let _client_tag = headers.get("x-demo-client").and_then(|v| v.to_str().ok()).unwrap_or("unknown");

    Ok(SearchResult { total: matches.len(), matches })
}

#[endpoint(auth = auth_handler, raw)]
fn version_blob() -> anyhow::Result<Vec<u8>> { Ok(b"milrouter-demo-v2\n".to_vec()) }

#[derive(Router)]
#[assets("./example/static")] // Optional.
                               // Serves static assets (relative to the file in which its invoked)
                               // If `MILROUTER_LOCAL` is set, will read from disk every request
                               // Otherwise, will load into LazyLock
#[html(super_awesome_html_generator)] // Optional.
#[middleware(RateLimitMiddleware, CorsMiddleware)]
pub enum DemoRouter {
    TheTime(EndpointTheTime), // `EndpointTheTime` is created by the #[endpoint] macro.
                              // It impls the traits required to make requests.
                              //
                              // `TheTime` can be named however youd like. It corresponds
                              //  to the underlying route name.
    Search(EndpointSearch),
    VersionBlob(EndpointVersionBlob),
}

#[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
pub async fn query_tools_demo(base_url: &str) -> anyhow::Result<(String, SearchResult, Vec<u8>)> {
    let mut headers = hyper::HeaderMap::new();
    headers.insert("x-demo-client", hyper::header::HeaderValue::from_static("readme-example"));

    let client = DemoRouter::client(base_url.to_string(), headers);

    let now = client.the_time(()).await?;
    let search = client
        .search(SearchQuery {
            needle: "or".to_string(),
            haystack: vec!["router".to_string(), "planet".to_string(), "orbit".to_string()],
        })
        .await?;
    let blob = client.version_blob(()).await?;

    Ok((now, search, blob))
}