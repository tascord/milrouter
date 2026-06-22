#![allow(unused)]
use std::ops::Not;

// Just dont want to gate even MORE things behind cfgs
use milrouter::{Endpoint, Router, anyhow, endpoint};

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