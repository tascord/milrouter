<img width="200" height="200" align="left" style="float: left; margin: 0 10px 0 0;" alt="Icon" src="https://github.com/tascord/milrouter/blob/main/Logo.svg?raw=true"> 

# Millennium Router

[![GitHub top language](https://img.shields.io/github/languages/top/tascord/milrouter?color=0072CE&style=for-the-badge)](#)
[![Crates.io Version](https://img.shields.io/crates/v/milrouter?style=for-the-badge)](https://crates.io/crates/milrouter)
[![docs.rs](https://img.shields.io/docsrs/milrouter?style=for-the-badge)](https://docs.rs/milrouter)

<br><br>

## What is?
A simple + relatively lightweight HTTP router for Rust, built on top of [hyper](https://crates.io/crates/hyper), and made to work well in [wasm](https://crates.io/crates/wasm_bindgen) world.

## How do?
A snippet from the [example router](./example/server/src/lib.rs):
```rust
#[endpoint(auth = auth_handler, idempotent = true)]
fn the_time() -> anyhow::Result<String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().to_string())
}

#[endpoint(auth = auth_handler)]
fn search(headers: hyper::HeaderMap, query: SearchQuery) -> anyhow::Result<SearchResult> {
    let needle = query.needle.to_lowercase();
    let matches = query
        .haystack
        .into_iter()
        .filter(|v| v.to_lowercase().contains(&needle))
        .collect::<Vec<_>>();

    let _client_tag = headers.get("x-demo-client");
    Ok(SearchResult { total: matches.len(), matches })
}

#[endpoint(auth = auth_handler, raw)]
fn version_blob() -> anyhow::Result<Vec<u8>> {
    Ok(b"milrouter-demo-v2\n".to_vec())
}

#[derive(Router)]
#[assets("./example/static")]    // Optional — embed & serve static files.
#[html(super_awesome_html_generator)]  // Optional — serve a fallback HTML generator.
pub enum DemoRouter {
    TheTime(EndpointTheTime),
    Search(EndpointSearch),
    VersionBlob(EndpointVersionBlob),
}
```

## Starting a server
`#[derive(Router)]` generates a `Router` impl with a `new()` constructor. Pass the instance to one of the built-in helpers:

```rust
// Single-thread local set — easiest for standalone binaries.
milrouter::serve_local("127.0.0.1:8080".parse().unwrap(), DemoRouter::new()).unwrap();

// Or, inside an existing Tokio runtime:
#[tokio::main]
async fn main() {
    milrouter::serve("127.0.0.1:8080".parse().unwrap(), DemoRouter::new()).await.unwrap();
}
```

## Endpoint macros
- `auth = your_auth_fn` (required): gate requests with your own async auth function.
- `idempotent = true` (optional): uses `PUT` instead of `POST`.
- `raw` (optional): endpoint returns `anyhow::Result<Vec<u8>>` and skips JSON/gzip.
- `stream` (optional): endpoint returns `anyhow::Result<milrouter::ResponseStream>`.

## Router attributes
- `#[assets("./static")]` — embed static files at compile time (served from `static/`). Set `MILROUTER_LOCAL` to read from disk instead.
- `#[html(my_html_fn)]` — register a fallback HTML generator for `/`.
- `#[middleware(Cors, RateLimit)]` — register stackable middleware (see below).

## Middleware
Implement the `Middleware` trait to hook into the request lifecycle. Both methods have default no-op implementations, so you only need to override the ones you care about.

- **`before`** — called before the endpoint. Return `Ok(Some(response))` to short-circuit the router (e.g. rate-limit rejection, CORS preflight). Return `Ok(None)` to continue.
- **`after`** — called on the response *after* the endpoint has run. Use it to inject headers that should appear on **every** response (CORS, Server, Request-Id, etc.).

Because `after` runs after the request body has been consumed by the endpoint, you should stash any request data you need in `self` during `before`.

### Example: Rate Limiting (stateful `before`)

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};

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
        if entries.len() >= self.limit { return false; }
        entries.push(now);
        true
    }
}

struct RateLimitMiddleware { limiter: RateLimiter }

impl RateLimitMiddleware {
    fn new() -> Self { Self { limiter: RateLimiter::new(10, Duration::from_secs(60)) } }
}

impl milrouter::Middleware for RateLimitMiddleware {
    fn before(
        &mut self,
        req: &hyper::Request<hyper::body::Incoming>,
    ) -> milrouter::futures::future::BoxFuture<'static, anyhow::Result<Option<hyper::Response<http_body_util::Full<bytes::Bytes>>>>> {
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
                let res = hyper::Response::builder()
                    .status(429)
                    .header("Content-Type", "text/plain")
                    .header("Retry-After", "60")
                    .body(http_body_util::Full::new(bytes::Bytes::from("Rate limit exceeded.")))
                    .unwrap();
                return Ok(Some(res));
            }
            Ok(None)
        })
    }
}
```

### Example: CORS (`before` for preflight + `after` for headers)

```rust
struct CorsMiddleware {
    origin: Option<String>,
}

impl CorsMiddleware {
    fn new() -> Self { CorsMiddleware { origin: None } }
}

impl milrouter::Middleware for CorsMiddleware {
    /// Captures the Origin and short-circuits on OPTIONS.
    fn before(
        &mut self,
        req: &hyper::Request<hyper::body::Incoming>,
    ) -> milrouter::futures::future::BoxFuture<'static, anyhow::Result<Option<hyper::Response<http_body_util::Full<bytes::Bytes>>>>> {
        self.origin = req.headers()
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        if req.method() == hyper::Method::OPTIONS {
            let origin = self.origin.as_deref().unwrap_or("*").to_string();
            Box::pin(async move {
                let res = hyper::Response::builder()
                    .status(200)
                    .header("Access-Control-Allow-Origin", origin.as_str())
                    .header("Access-Control-Allow-Methods", "GET, POST, PUT, OPTIONS")
                    .header("Access-Control-Allow-Headers", "Content-Type, Authorization")
                    .body(http_body_util::Full::new(bytes::Bytes::new()))
                    .unwrap();
                Ok(Some(res))
            })
        } else {
            Box::pin(async { Ok(None) })
        }
    }

    /// Adds CORS headers to every response that makes it past `before`.
    fn after(
        &mut self,
        res: &mut hyper::Response<()>,
    ) -> milrouter::futures::future::BoxFuture<'static, anyhow::Result<()>> {
        let origin = self.origin.as_deref().unwrap_or("*").to_string();
        res.headers_mut().insert(
            "Access-Control-Allow-Origin",
            hyper::header::HeaderValue::from_str(&origin)
                .unwrap_or_else(|_| hyper::header::HeaderValue::from_static("*")),
        );
        res.headers_mut().insert(
            "Access-Control-Allow-Methods",
            hyper::header::HeaderValue::from_static("GET, POST, PUT, OPTIONS"),
        );
        Box::pin(async move { Ok(()) })
    }
}
```

Attach middleware to your router with the derive attribute:

```rust
#[derive(Router)]
#[middleware(RateLimitMiddleware, CorsMiddleware)]
pub enum DemoRouter {
    ...
}
```

## Query tools
`#[derive(Router)]` now generates a typed client for non-wasm targets:
```rust
let mut headers = hyper::HeaderMap::new();
headers.insert("x-demo-client", hyper::header::HeaderValue::from_static("readme"));

let client = DemoRouter::client("http://127.0.0.1:40000".to_string(), headers);
let now = client.the_time(()).await?;
let found = client.search(SearchQuery {
    needle: "or".to_string(),
    haystack: vec!["router".to_string(), "planet".to_string(), "orbit".to_string()],
}).await?;
let version = client.version_blob(()).await?;
```

In wasm, use the request helper from [example/wasm/src/lib.rs](./example/wasm/src/lib.rs):
```rust
milrouter::wasm::request(
    server::EndpointSearch,
    server::SearchQuery {
        needle: "or".to_string(),
        haystack: vec!["router".to_string(), "planet".to_string(), "orbit".to_string()],
    }
).await
```

### shell, if you're feeling frisky:
JSON endpoints are transported as GZipped JSON; raw endpoints are plain bytes.
```sh
curl http://localhost:40000/the_time -X put --output - --compressed
# 12345678901234
curl http://localhost:40000/search -X post \
  -H 'content-type: application/json' \
  -H 'x-demo-client: shell' \
  --data '{"needle":"or","haystack":["router","planet","orbit"]}' \
  --output - --compressed

curl http://localhost:40000/version_blob -X post --output -
# milrouter-demo-v2
```
