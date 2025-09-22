<img width="200" height="200" align="left" style="float: left; margin: 0 10px 0 0;" alt="Icon" src="https://github.com/tascord/milrouter/blob/main/icon.png?raw=true"> 

# Millennium Router
## 
<h3 style="color: cornflowerblue; rotate: -5deg; margin: -60px 0px 30px;">Now with Stage 3a water restrictions!</h3>

[![GitHub top language](https://img.shields.io/github/languages/top/tascord/milrouter?color=0072CE&style=for-the-badge)](#)
[![Crates.io Version](https://img.shields.io/crates/v/milrouter?style=for-the-badge)](https://crates.io/crates/milrouter)
[![docs.rs](https://img.shields.io/docsrs/milrouter?style=for-the-badge)](https://docs.rs/ptv)

<br><br>

## What is?
A simple + relatively lightweight HTTP router for Rust, built on top of [hyper](https://crates.io/crates/hyper), and made to work well in [wasm](https://crates.io/crates/wasm_bindgen) world.

## How do?
A snippet from the [example router](./example/server/src/lib.rs):
```rust
#[endpoint(auth = auth_handler)]
fn the_time() -> anyhow::Result<String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().to_string())
}

#[derive(Router)]
pub enum DemoRouter {
    TheTime(EndpointTheTime), 
}
```

### [wasm](./wasm/src/lib.rs):
```rust
milrouter::wasm::request(server::EndpointTheTime, ()).await // (or .as_mutable)
// 12345678901234
```

### shell, if you're feeling frisky:
We're just transporting GZipped JSON
```sh
curl http://localhost:40000/the_time -X put --output - --compressed
# 12345678901234
```