use {
    heck::AsSnekCase,
    crate::helpers::{parse_attrs, preamble},
    proc_macro::{Span, TokenStream},
    quote::{ToTokens, format_ident, quote},
    syn::{parse_macro_input, DeriveInput},
};

pub fn expand_router(item: TokenStream) -> TokenStream {
    let (input, name, data) = preamble(parse_macro_input!(item as DeriveInput));
    let (html, local_assets, mware) = parse_attrs(input.clone());

    let client_name = format_ident!("{}Client", name);

    let first_variant = data.variants.first().map(|v| {
        let ident = &v.ident;
        let inner = v.fields.iter().next().map(|ty| ty.ty.clone());
        quote::quote!(#name::#ident(#inner))
    }).unwrap_or_else(|| quote::quote!(panic!("Router enum cannot be empty")));

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
            (stringify!(#path), i) if i == #inner::is_idempotent() => {
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
                        let bytes = req.collect().await.unwrap_or_else(|e| panic!("Failed to read incoming bytes for {}: {e}", stringify!(#inner_name))).to_bytes();
                        let body_str = String::from_utf8_lossy(&bytes[..]).to_string();
                        std::boxed::Box::new(
                            milrouter::serde_json::from_str::<<#inner as milrouter::Endpoint<_>>::Data>(&body_str)
                                .unwrap_or_else(|e| panic!("Failed to deserialize body for {}: {e}", stringify!(#inner_name)))
                        )
                    }
                };

                let body: <#inner as milrouter::Endpoint<_>>::Data = *body.downcast::<<#inner as milrouter::Endpoint<_>>::Data>().unwrap();

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
                            use std::any::Any;
                            let raw: std::boxed::Box<dyn Any> = std::boxed::Box::new(response);
                            let bytes: Vec<u8> = *raw.downcast::<Vec<u8>>()
                                .expect("Internal error: raw endpoint handler did not return Vec<u8> as expected. Ensure the endpoint function returns anyhow::Result<Vec<u8>>.");

                            milrouter::tracing::info!(concat!("[+] 200 Ok (raw) /", stringify!(#path)));
                            milrouter::hyper::Response::builder()
                                .status(200)
                                .body(milrouter::Body::from(bytes.as_slice()).boxed())
                                .unwrap()
                        } else {
                            let bytes = milrouter::serde_json::to_vec(&response).unwrap_or_else(|e| panic!("Failed to serialize response for {}: {e}", stringify!(#inner_name)));

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
            },
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

                    let is_gzipped = resp
                        .headers()
                        .get("content-encoding")
                        .and_then(|value| value.to_str().ok())
                        .map(|value| {
                            value
                                .split(',')
                                .any(|encoding| encoding.trim().eq_ignore_ascii_case("gzip"))
                        })
                        .unwrap_or(false);

                    let bytes = resp.bytes().await?;
                    let bytes = if is_gzipped {
                        let mut decompressed = Vec::new();
                        milrouter::gz_decompress(bytes.as_ref(), &mut decompressed)?;
                        milrouter::bytes::Bytes::from(decompressed)
                    } else {
                        bytes
                    };

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
        Some(_local_assets) => quote::quote! {
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

    let mware_idents: Vec<proc_macro2::Ident> = mware.as_ref().map(|mware_ts| {
        mware_ts.clone().into_iter().filter_map(|tt| match tt {
            proc_macro2::TokenTree::Ident(i) => Some(i),
            _ => None,
        }).collect()
    }).unwrap_or_default();

    let mware_fn = if mware_idents.is_empty() {
        quote! {}
    } else {
        quote! {
            async fn __middleware_chain(req: &milrouter::hyper::Request<milrouter::hyper::body::Incoming>, middlewares: &mut Vec<std::boxed::Box<dyn milrouter::Middleware>>) -> milrouter::anyhow::Result<Option<milrouter::hyper::Response<milrouter::http_body_util::Full<milrouter::bytes::Bytes>>>> {
                use milrouter::Middleware;
                for mw in middlewares.iter_mut() {
                    match mw.route(req).await {
                        Ok(Some(response)) => return Ok(Some(response)),
                        Ok(None) => {}
                        Err(e) => {
                            milrouter::tracing::error!("[-] 500 Middleware Error: {}", e);
                            let body = milrouter::http_body_util::Full::new(milrouter::bytes::Bytes::from(e.to_string()));
                            return Ok(Some(milrouter::hyper::Response::builder()
                                .status(500)
                                .body(body)
                                .unwrap()));
                        }
                    }
                }
                Ok(None)
            }
        }
    };

    let mware_impl = if mware_idents.is_empty() {
        quote! {
            fn middleware(&self) -> Vec<std::boxed::Box<dyn milrouter::Middleware>> {
                vec![]
            }
        }
    } else {
        quote! {
            fn middleware(&self) -> Vec<std::boxed::Box<dyn milrouter::Middleware>> {
                vec! #( (Box::new(#mware_idents::new()) as Box<dyn milrouter::Middleware>) )*
            }
        }
    };

    let mware_call = if mware_idents.is_empty() {
        quote! {}
    } else {
        quote! {
            let mut middlewares = vec![#( (std::boxed::Box::new(#mware_idents::new()) as std::boxed::Box<dyn milrouter::Middleware>) )*];
            if let Ok(Some(response)) = __middleware_chain(&req, &mut middlewares).await {
                let headers = response.headers().clone();
                let body = response.into_body().boxed();
                let mut res = milrouter::hyper::Response::new(body);
                *res.headers_mut() = headers;
                return Ok(res);
            }
        }
    };

    let ts = TokenStream::from(quote::quote! {
        #mware_fn

        #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
        static __ASSETS: std::sync::LazyLock<std::collections::BTreeMap::<String, (String, &'static [u8])>> = std::sync::LazyLock::new(|| {
            use std::io::Read;
            let mut assets = std::collections::BTreeMap::<String, (String, &'static [u8])>::new();
            #(#inserts)*
            assets
        });

        #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
        pub struct __marker(pub ());

        #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
        impl #name {
            pub fn new() -> #name { #first_variant }

            pub async fn route(req: milrouter::hyper::Request<milrouter::hyper::body::Incoming>) -> std::result::Result<milrouter::hyper::Response<milrouter::MilBody>, std::convert::Infallible> {
                use milrouter::http_body_util::BodyExt;

                let path = req.uri().path().to_string();
                let path = path.strip_prefix("/").map(|v| v.to_string()).unwrap_or(path);
                let path = path.strip_prefix("static/").map(|v| v.to_string()).unwrap_or(path);
                let headers = req.headers().clone();
                let method = req.method().clone();
                let is_idempotent = req.method().is_idempotent();

                #mware_call

                if method == milrouter::hyper::Method::GET {
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
                    match (path.as_str(), is_idempotent) {
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

            #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
            pub fn client(host: String, headers: milrouter::hyper::HeaderMap) -> #client_name {
                #client_name { host, headers }
            }
        }

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

        impl milrouter::Router for #name {
            fn route(&self, req: milrouter::hyper::Request<milrouter::hyper::body::Incoming>) -> std::pin::Pin<std::boxed::Box<dyn std::future::Future<Output = std::result::Result<milrouter::hyper::Response<milrouter::MilBody>, std::convert::Infallible>> + std::marker::Send + 'static>> {
                Box::pin(#name::route(req))
            }

            #mware_impl
        }

        impl milrouter::Router for __marker {
            fn route(&self, _: milrouter::hyper::Request<milrouter::hyper::body::Incoming>) -> std::pin::Pin<std::boxed::Box<dyn std::future::Future<Output = std::result::Result<milrouter::hyper::Response<milrouter::MilBody>, std::convert::Infallible>> + std::marker::Send + 'static>> {
                Box::pin(async move {
                    milrouter::tracing::warn!("[*] 418 I'm a teapot");
                    Ok(milrouter::hyper::Response::builder()
                        .status(418)
                        .body(milrouter::Body::from(b"I'm a teapot".as_slice()).boxed())
                        .unwrap())
                })
            }

            fn middleware(&self) -> Vec<std::boxed::Box<dyn milrouter::Middleware>> {
                vec![]
            }
        }

        impl std::fmt::Display for __marker {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "__marker")
            }
        }

        #(#into_routers)*

    });

    ts
}