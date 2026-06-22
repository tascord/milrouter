use {
    crate::helpers::parse_attrs,
    heck::{AsPascalCase, AsSnekCase},
    helpers::{RouteInfo, get_inner_type, parse_fn_args, preamble, unit},
    proc_macro::{Span, TokenStream},
    quote::{ToTokens, format_ident, quote},
    syn::{DeriveInput, FnArg, parse_macro_input},
};

#[macro_use]
mod helpers;

/// ### Arguments
/// Takes the following arguments (comma-separated):
///
/// - `auth = fn` — **Required**<br>
///   Function `async fn(HeaderMap) -> anyhow::Result<C>` that gates the request.<br>
///   Use [`milrouter::all_aboard`] to accept all requests (unit client).
///
/// - `is_idempotent` — **Optional** (default `false`)<br>
///   Maps to HTTP `PUT` when true, `POST` when false.
///
/// - `raw` — **Optional**<br>
///   Return type must be `anyhow::Result<Vec<u8>>`.  The bytes are sent as-is
///   (no JSON encoding, no gzip).
///
/// - `stream` — **Optional**<br>
///   Return type must be `anyhow::Result<milrouter::ResponseStream>`.  The body
///   is sent as a chunked streaming response.  Can be combined with `raw`.
///
/// ### Example:
/// ```rust
/// #[endpoint(auth = auth_handler)]
/// async fn greet(_: (), name: String) -> anyhow::Result<String> {
///     Ok(format!("Hello, {name}!"))
/// }
/// ```
#[proc_macro_attribute]
pub fn endpoint(annot: TokenStream, item: TokenStream) -> TokenStream {
    let it = item.clone();
    let meta = parse_macro_input!(it as syn::ItemFn);

    let name = meta.sig.clone().ident;
    let ret = meta.sig.clone().output;
    let block = meta.block;

    let args = parse_fn_args(
        meta.sig
            .inputs
            .iter()
            .map(|a| {
                let a = match a {
                    FnArg::Typed(t) => t,
                    _ => panic!("Unexpected self type in endpoint"),
                };

                let ident = match *a.clone().pat {
                    syn::Pat::Ident(pat_ident) => pat_ident.ident,
                    _ => unreachable!(),
                };

                let ty = *a.clone().ty;

                (ident, ty)
            })
            .collect::<Vec<_>>(),
    );

    let info = err!(RouteInfo::parse(annot.into()));
    let (idempotent, auth, is_raw, is_stream) = (info.is_idempotent, info.auth, info.raw, info.stream);

    let method = match idempotent {
        true => "PUT",
        false => "POST",
    };

    let inner_ret = match meta.sig.clone().output {
        syn::ReturnType::Type(_, ty) => *ty,
        _ => unreachable!(),
    };

    let inner_ret = err!(get_inner_type(inner_ret.clone()).map_err(|e| {
        syn::Error::new_spanned(
            ret.to_token_stream(),
            format!("Unexpected return type (should be anyhow::Result<T>).\n{e}"),
        )
    }));

    let struct_name = quote::format_ident!("Endpoint{}", AsPascalCase(name.to_string()).to_string());
    let name_str = name.to_string();

    let data = args.clone().input.1;
    let client_type = args.client.clone().map(|c| c.1).unwrap_or(unit());
    let args_tokens = args.to_tokens();

    // Non-stream endpoints implement ClientEndpoint so the generated router
    // client can call them with proper typed decode.
    let client_endpoint_impl = if is_stream {
        quote! {}
    } else if is_raw {
        // raw: Returns = Vec<u8>; bytes are returned verbatim
        quote! {
            #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
            impl milrouter::TypedEndpoint for #struct_name {
                type Client = #client_type;
            }

            #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
            impl milrouter::ClientEndpoint<#client_type> for #struct_name {
                fn decode_response(bytes: milrouter::bytes::Bytes) -> milrouter::anyhow::Result<Vec<u8>> {
                    Ok(bytes.to_vec())
                }
            }
        }
    } else {
        // normal: JSON-deserialise
        quote! {
            #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
            impl milrouter::TypedEndpoint for #struct_name {
                type Client = #client_type;
            }

            #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
            impl milrouter::ClientEndpoint<#client_type> for #struct_name
            where
                #inner_ret: milrouter::serde::de::DeserializeOwned,
            {
                fn decode_response(bytes: milrouter::bytes::Bytes) -> milrouter::anyhow::Result<#inner_ret> {
                    milrouter::serde_json::from_slice(&bytes).map_err(|e| milrouter::anyhow::anyhow!(e))
                }
            }
        }
    };

    // ServerEndpoint impl: stream vs raw vs normal
    let server_endpoint_impl = if is_stream {
        quote! {
            #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
            impl milrouter::ServerEndpoint<#client_type> for #struct_name {
                fn auth() -> milrouter::AsyncHandler<milrouter::hyper::HeaderMap, milrouter::anyhow::Result<#client_type>> {
                    Box::new(move |i: milrouter::hyper::HeaderMap| Box::pin(#auth(i)))
                }

                fn handler() -> milrouter::AsyncHandler3<#client_type, milrouter::hyper::HeaderMap, Self::Data, milrouter::anyhow::Result<Self::Returns>> {
                    // Unreachable for stream endpoints; stream_handler() is used instead.
                    Box::new(move |_, _, _| Box::pin(async { unreachable!("handler() called on a stream endpoint") }))
                }

                fn stream_handler() -> Option<milrouter::AsyncHandler3<#client_type, milrouter::hyper::HeaderMap, Self::Data, milrouter::anyhow::Result<milrouter::ResponseStream>>> {
                    Some(Box::new(move |c: #client_type, h: milrouter::hyper::HeaderMap, d: Self::Data| Box::pin(#name(c, h, d))))
                }
            }
        }
    } else {
        let is_raw_val = is_raw;
        quote! {
            #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
            impl milrouter::ServerEndpoint<#client_type> for #struct_name {
                fn auth() -> milrouter::AsyncHandler<milrouter::hyper::HeaderMap, milrouter::anyhow::Result<#client_type>> {
                    Box::new(move |i: milrouter::hyper::HeaderMap| Box::pin(#auth(i)))
                }

                fn handler() -> milrouter::AsyncHandler3<#client_type, milrouter::hyper::HeaderMap, Self::Data, milrouter::anyhow::Result<Self::Returns>> {
                    Box::new(move |i: #client_type, i2: milrouter::hyper::HeaderMap, i3: Self::Data| Box::pin(#name(i, i2, i3)))
                }

                fn is_raw() -> bool { #is_raw_val }
            }
        }
    };

    quote::quote! {
        #[doc = concat!("Endpoint struct for [`", stringify!(#name), "`]  \n@ ", stringify!(#method), " → `", stringify!(#struct_name), "::Data` ([`", stringify!(#ret), "`])")]
        #[derive(Clone)]
        pub struct #struct_name;

        impl milrouter::Endpoint<#client_type> for #struct_name {
            type Data = #data;
            type Returns = #inner_ret;

            fn is_idempotent() -> bool { #idempotent }
            fn path() -> &'static str { #name_str }
        }

        #server_endpoint_impl

        #client_endpoint_impl

        #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
        pub async fn #name(#args_tokens) #ret #block

    }
    .into()
}

/// Apply to an enum. <br>
/// Variants' snake_case names are used as paths, and inner type's used as endpoint handlers.
/// ### Example:
/// ```rust
/// #[derive(Router)]
/// #[assets("./example/static")]
/// #[html(super_awesome_html_generator)]
/// pub enum DemoRouter {
///     Greet(EndpointGreet),
/// }
/// ```
#[proc_macro_derive(Router, attributes(assets, html))]
pub fn router(item: TokenStream) -> TokenStream {
    let (input, name, data) = preamble(parse_macro_input!(item as DeriveInput));
    let (html, local_assets) = parse_attrs(input.clone());

    let client_name = format_ident!("{}Client", name);

    let paths: Result<Vec<proc_macro2::TokenStream>, syn::Error> = data.variants.iter().map(|variant| {

        let path = format_ident!("{}", AsSnekCase(variant.ident.to_string()).to_string());
        let inner = variant.fields.iter()
            .next()
            .map(|ty| ty.ty.clone())
            .ok_or(syn::Error::new_spanned(
                variant.to_token_stream(),
                format!("No endpoint specified for {}", variant.ident)
            ))?;

        let inner_name = &variant.ident;

        Ok(quote::quote! {
            (stringify!(#path), i) if i == #inner::is_idempotent() => ({
                let auth = <#inner as milrouter::ServerEndpoint<_>>::auth();

                let error_res = |e: String, code: u16, label: &'static str| {
                    milrouter::tracing::info!("[-] {code} {label} /{}", stringify!(#path));
                    milrouter::hyper::Response::builder()
                        .status(code)
                        .body(
                            milrouter::Body::from(format!(
                                "You aren't authorised to access this endpoint\n{e}"
                            ))
                            .boxed()
                        )
                        .unwrap()
                };

                let client = match auth(headers.clone()).await {
                    Ok(c) => c,
                    Err(e) => return error_res(e.to_string(), 401, "Unauthorised"),
                };

                let body: std::boxed::Box<dyn std::any::Any> = match std::any::type_name::<<#inner as milrouter::Endpoint<_>>::Data>() {
                    "()" => std::boxed::Box::new(()),
                    _ => {
                        let bytes = req.collect().await.unwrap_or_else(|_| panic!("Failed to read incoming bytes for {}", stringify!(#inner_name))).to_bytes();
                        let body_str = String::from_utf8_lossy(&bytes[..]).to_string();
                        std::boxed::Box::new(
                            milrouter::serde_json::from_str::<<#inner as milrouter::Endpoint<_>>::Data>(&body_str)
                                .unwrap_or_else(|e| panic!("Failed to deserialise body for {}: {e}", stringify!(#inner_name)))
                        )
                    }
                };

                let body: <#inner as milrouter::Endpoint<_>>::Data = *body.downcast::<<#inner as milrouter::Endpoint<_>>::Data>().unwrap();

                // Streaming endpoint
                if let Some(stream_handler) = <#inner as milrouter::ServerEndpoint<_>>::stream_handler() {
                    return match stream_handler(client, headers, body).await {
                        Ok(stream) => {
                            milrouter::tracing::info!(concat!("[+] 200 Ok (stream) /", stringify!(#path)));
                            milrouter::hyper::Response::builder()
                                .status(200)
                                .body(milrouter::stream_to_body(stream))
                                .unwrap()
                        }
                        Err(e) => {
                            milrouter::tracing::warn!(concat!("[-] 400 Bad Request (stream) /", stringify!(#path)));
                            milrouter::hyper::Response::builder()
                                .status(400)
                                .body(milrouter::Body::from(e.to_string()).boxed())
                                .unwrap()
                        }
                    };
                }

                let handler = <#inner as milrouter::ServerEndpoint<_>>::handler();

                match handler(client, headers, body).await {
                    Ok(response) => {
                        if <#inner as milrouter::ServerEndpoint<_>>::is_raw() {
                            // Raw endpoint: treat Returns as Vec<u8> and send bytes as-is
                            use std::any::Any;
                            let raw: std::boxed::Box<dyn Any> = std::boxed::Box::new(response);
                            let bytes: Vec<u8> = *raw.downcast::<Vec<u8>>()
                                .expect("raw endpoint must return Vec<u8>");

                            milrouter::tracing::info!(concat!("[+] 200 Ok (raw) /", stringify!(#path)));
                            milrouter::hyper::Response::builder()
                                .status(200)
                                .body(milrouter::Body::from(bytes.as_slice()).boxed())
                                .unwrap()
                        } else {
                            let bytes = milrouter::serde_json::to_vec(&response).unwrap_or_else(|e| panic!("Failed to serialise response for {}: {e}", stringify!(#inner_name)));

                            let mut compressed_file = Vec::new();
                            milrouter::gz_compress(bytes.as_slice(), &mut compressed_file).unwrap();

                            milrouter::tracing::info!(concat!("[+] 200 Ok /", stringify!(#path)));
                            milrouter::hyper::Response::builder()
                                .status(200)
                                .header("Content-Encoding", "gzip")
                                .body(milrouter::Body::from(compressed_file.as_slice()).boxed())
                                .unwrap()
                        }
                    },
                    Err(e) => {
                        milrouter::tracing::warn!(concat!("[-] 400 Bad Request /", stringify!(#path)));
                        milrouter::hyper::Response::builder()
                            .status(400)
                            .body(milrouter::Body::from(e.to_string()).boxed())
                            .unwrap()
                    }
                }
            }),
        })
    }).collect();

    let paths: Vec<proc_macro2::TokenStream> = err!(paths);

    let into_routers: Result<Vec<proc_macro2::TokenStream>, syn::Error> = data
        .variants
        .iter()
        .map(|variant| {
            let ident = variant.fields.iter().next().map(|ty| ty.ty.clone()).ok_or(syn::Error::new_spanned(
                variant.to_token_stream(),
                format!("No endpoint specified for {}", variant.ident),
            ))?;

            let variant = variant.ident.clone();

            Ok(quote::quote! {
                impl milrouter::IntoRouter<#name> for #ident {
                    fn router(self) -> #name {
                        #name::#variant(#ident)
                    }
                }
            })
        })
        .collect();

    let into_routers: Vec<proc_macro2::TokenStream> = err!(into_routers);

    let as_paths = data
        .variants
        .iter()
        .map(|variant| {
            let ident = variant.ident.clone();
            let snake = heck::AsSnekCase(variant.ident.to_string()).to_string();
            quote::quote! {
               Self::#ident(..) => f.write_str(#snake),
            }
        })
        .collect::<Vec<_>>();

    // Client method generation: one async method per non-stream variant.
    // Stream variants do not get a client method.
    let client_methods: Vec<proc_macro2::TokenStream> = data
        .variants
        .iter()
        .map(|variant| {
            let inner = variant.fields.iter().next().map(|ty| ty.ty.clone()).unwrap();
            let method_name = format_ident!("{}", AsSnekCase(variant.ident.to_string()).to_string());

            quote::quote! {
                pub async fn #method_name(
                    &self,
                    data: <#inner as milrouter::Endpoint<<#inner as milrouter::TypedEndpoint>::Client>>::Data,
                ) -> milrouter::anyhow::Result<<#inner as milrouter::Endpoint<<#inner as milrouter::TypedEndpoint>::Client>>::Returns>
                where
                    #inner: milrouter::TypedEndpoint + milrouter::ClientEndpoint<<#inner as milrouter::TypedEndpoint>::Client>,
                    <#inner as milrouter::Endpoint<<#inner as milrouter::TypedEndpoint>::Client>>::Data: milrouter::serde::Serialize,
                {
                    let url = format!("{}/{}", self.host, <#inner as milrouter::Endpoint<<#inner as milrouter::TypedEndpoint>::Client>>::path());
                    let method = if <#inner as milrouter::Endpoint<<#inner as milrouter::TypedEndpoint>::Client>>::is_idempotent() {
                        milrouter::reqwest::Method::PUT
                    } else {
                        milrouter::reqwest::Method::POST
                    };
                    let resp = milrouter::reqwest::Client::new()
                        .request(method, &url)
                        .headers(self.headers.clone())
                        .json(&data)
                        .send()
                        .await?;
                    let bytes = resp.bytes().await?;
                    <#inner as milrouter::ClientEndpoint<<#inner as milrouter::TypedEndpoint>::Client>>::decode_response(bytes)
                }
            }
        })
        .collect();

    let walkdir = |p: std::path::PathBuf| {
        walkdir::WalkDir::new(&p)
            .into_iter()
            .filter_map(|e| match e {
                Err(_) => None,
                Ok(f) => f.metadata().unwrap().is_file().then_some(f),
            })
            .map(move |entry| {
                let route =
                    entry.path().display().to_string().strip_prefix(&format!("{}/", p.display())).unwrap().to_string();

                let path = entry.path().display().to_string();

                let mime = mime_guess::from_path(route.clone()).first_or_text_plain().to_string();
                quote::quote! {
                    assets.insert(#route.to_string(), (#mime.to_string(), include_bytes!(#path)));
                }
            })
    };

    let inserts = match local_assets.clone() {
        Some(v) => {
            let root = Span::call_site().local_file().unwrap_or_default();
            walkdir(root.join(&v)).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };

    let default_route_case = match html {
        None => quote::quote!(),
        Some(html) => quote::quote! {
            else if path.is_empty() {
                milrouter::tracing::info!("[#] 200 Ok (HTML) /{}", path);
                return Ok(
                    milrouter::hyper::Response::builder()
                        .status(200)
                        .header("Content-Type", "text/html")
                        .body(milrouter::Body::from(#html()).boxed())
                        .unwrap()
                )
            }
        },
    };

    let assets_serving = match local_assets.clone() {
        Some(local_assets) => quote::quote! {
             if let Some(file) = __ASSETS.get(&path) {
                milrouter::tracing::info!("[#] 200 Ok (File) /{}", path);
                return Ok(
                    milrouter::hyper::Response::builder()
                        .status(200)
                        .header("Content-Type", file.0.to_string())
                        .header("Content-Encoding", "gzip")
                        .body(match std::env::var("MILROUTER_LOCAL").is_ok() {
                            false => {
                                let mut compressed_file = Vec::new();
                                milrouter::gz_compress(file.1, &mut compressed_file).unwrap();
                                milrouter::Body::from(compressed_file.as_slice()).boxed()
                            },
                            true => {
                                use std::io::Read;
                                let mut byt = Vec::new();

                                let _ = std::fs::File::open(std::path::PathBuf::from(#local_assets).join(&path)).and_then(|mut f| f.read_to_end(&mut byt));
                                let mut compressed_file = Vec::new();
                                milrouter::gz_compress(byt.as_slice(), &mut compressed_file).unwrap();
                                milrouter::Body::from(compressed_file.as_slice()).boxed()
                            }
                        })
                        .unwrap()
                )
            }
        },
        _ => quote::quote!(),
    };

    let el = if assets_serving.is_empty() && default_route_case.is_empty() {
        quote! {}
    } else {
        quote! { else }
    };

    let ts = TokenStream::from(quote::quote! {
        #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
        static __ASSETS: std::sync::LazyLock<std::collections::BTreeMap::<String, (String, &'static [u8])>> = std::sync::LazyLock::new(|| {
            use std::io::Read;
            let mut assets = std::collections::BTreeMap::<String, (String, &'static [u8])>::new();
            #(#inserts)*
            assets
        });

        #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
        impl #name {
            pub async fn route(req: milrouter::hyper::Request<milrouter::hyper::body::Incoming>) -> std::result::Result<milrouter::hyper::Response<milrouter::MilBody>, std::convert::Infallible> {
                use milrouter::http_body_util::BodyExt;

                let path = req.uri().path().to_string();
                let path = path.strip_prefix("/").map(|v| v.to_string()).unwrap_or(path);
                let path = path.strip_prefix("static/").map(|v| v.to_string()).unwrap_or(path);
                let headers = req.headers().clone();

                if req.method() == milrouter::hyper::Method::GET {
                    #assets_serving
                    #default_route_case
                    #el {
                        milrouter::tracing::warn!("[#] 404 Not Found /{}", path);
                        return Ok(
                            milrouter::hyper::Response::builder()
                                .status(404)
                                .body(milrouter::Body::default().boxed())
                                .unwrap()
                        )
                    }
                }

                Ok(match milrouter::tokio::task::spawn(async move {
                    match (path.as_str(), req.method().is_idempotent()) {
                        #(#paths)*
                        path => {
                            milrouter::tracing::info!("[?] 404 Not Found /{}", path.0);
                            milrouter::hyper::Response::builder()
                                .status(404)
                                .body(milrouter::Body::default().boxed())
                                .unwrap()
                        }
                    }
                }).await {
                    Ok(inner) => inner,
                    Err(err) => {
                        let err = err.into_panic();
                        let value = err
                            .downcast_ref::<String>()
                            .cloned()
                            .or(err.downcast_ref::<&str>().map(|s| s.to_string()))
                            .unwrap_or_else(|| "[Unexpected Error]".to_string());

                        milrouter::tracing::error!("[-] 500 Internal Server Error: {value}");
                        milrouter::hyper::Response::builder()
                            .status(500)
                            .body(milrouter::Body::from(value).boxed())
                            .unwrap()
                    }
                })
            }

            /// Build a typed HTTP client targeting this router at `host`.
            ///
            /// All requests will include the provided `headers`.
            ///
            /// ```no_run
            /// let client = MyRouter::client("http://localhost:8080".into(), Default::default());
            /// let result = client.my_endpoint(input).await?;
            /// ```
            #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
            pub fn client(host: String, headers: milrouter::hyper::HeaderMap) -> #client_name {
                #client_name { host, headers }
            }
        }

        /// Typed HTTP client for [`#name`].  Obtain via [`#name::client`].
        #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
        pub struct #client_name {
            host: String,
            headers: milrouter::hyper::HeaderMap,
        }

        #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
        impl #client_name {
            #(#client_methods)*
        }

        impl std::fmt::Display for #name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    #(#as_paths)*
                }
            }
        }

        impl milrouter::Router for #name {}

        #(#into_routers)*

    });

    // dbg!(ts.to_string());
    ts
}

// For docs

/// __Optional__
///  
/// Serves static assets (relative to the file in which its invoked)
///
/// If `MILROUTER_LOCAL` is set, will read from disk every request,
/// otherwise, will load into LazyLock
#[proc_macro_attribute]
pub fn assets(_: TokenStream, i: TokenStream) -> TokenStream { i }

/// __Optional__
///  
/// Serves static HTML governed from a function
///
/// If this is not provided, `/` will give a `400`
#[proc_macro_attribute]
pub fn html(_: TokenStream, i: TokenStream) -> TokenStream { i }

