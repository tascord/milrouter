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
Implement the `Middleware` trait to intercept requests before they reach your endpoints. Returning `Ok(Some(response))` short-circuits the router (useful for CORS, rate-limiting, logging, etc.). Returning `Ok(None)` lets the request continue.

```rust
struct CorsMiddleware;

impl CorsMiddleware {
    fn new() -> Self { CorsMiddleware }
}

impl milrouter::Middleware for CorsMiddleware {
    fn route(
        &mut self,
        req: &hyper::Request<hyper::body::Incoming>,
    ) -> milrouter::futures::future::BoxFuture<'static, anyhow::Result<Option<hyper::Response<http_body_util::Full<bytes::Bytes>>>>> {
        let origin = req.headers()
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("*")
            .to_string();

        Box::pin(async move {
            let res = hyper::Response::builder()
                .header("Access-Control-Allow-Origin", origin.as_str())
                .header("Access-Control-Allow-Methods", "GET, POST, PUT, OPTIONS")
                .body(http_body_util::Full::new(bytes::Bytes::new()))
                .unwrap();
            Ok(Some(res))
        })
    }
}
```

Attach it to your router with the derive attribute:

```rust
#[derive(Router)]
#[middleware(CorsMiddleware)]
pub enum DemoRouter {
    ...
}
```

Multiple middlewares are executed in declaration order:

```rust
#[derive(Router)]
#[middleware(CorsMiddleware, RateLimitMiddleware)]
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
