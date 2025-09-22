#![allow(unused)]
use std::ops::Not;

// Just dont want to gate even MORE things behind cfgs
use milrouter::{Endpoint, Router, anyhow, endpoint};

const HTML: &str = include_str!("../../static/index.html");
fn super_awesome_html_generator() -> String { HTML.to_string() }

pub async fn auth_handler(headers: hyper::HeaderMap) -> anyhow::Result<()> {
    headers.contains_key("evil").not().then_some(()).ok_or(anyhow::anyhow!("Evil request detected."))
}

#[endpoint(
    is_idempotent = false, // Optional.
                           // Idempotency is defalted to false
                           // Providing `is_idempotent` is sufficient
                           // See HTTP spec (https://datatracker.ietf.org/doc/html/rfc7231#section-4.2.2)

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
}