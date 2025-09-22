use {
    crate::helpers::parse_attrs,
    heck::{AsPascalCase, AsSnekCase},
    helpers::{RouteInfo, get_inner_type, parse_fn_args, preamble, unit},
    proc_macro::{Span, TokenStream},
    quote::{ToTokens, format_ident},
    syn::{DeriveInput, FnArg, parse_macro_input},
};

#[macro_use]
mod helpers;

/// ### Arguments
/// Takes 2 k=v arguments:
/// - `is_idempotent` — __Optional__<br>
///   Idempotency is defalted to false<br>
///   Providing `is_idempotent` is sufficient (no `= true` needed)<br>
///   See HTTP spec (https://datatracker.ietf.org/doc/html/rfc7231#section-4.2.2)
/// 
/// 
/// - `auth` — __Required__<br>
///   Function to determine which request are allowed through based on headers.<br>
///   The inner type returned (e.g unit in this case) is
///   passed to the `client` variable of the endpoint, if it exists
/// 
/// ### Function Signature
/// - #### Parameters (All optional):
///   - `client`<br>
///     Type derived from headers via `auth`<br>
///   - param (name derived from use)<br>
///     Type must be serializable, and representable in JSON (via serde-json).<br>
///     If undefined, becomes unit `()` 
/// 
/// ### Example:
/// ```rust
/// #[endpoint(is_idempotent = false, auth = auth_handler)]
/// fn route(client: (), param1: ()) -> anyhow::Result<String> {}
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
    let (idempotent, auth) = (info.is_idempotent, info.auth);

    let method = match idempotent {
        true => "PUT",
        false => "POST",
    };

    let inner_ret = match meta.sig.clone().output {
        syn::ReturnType::Type(_, ty) => *ty,
        _ => unreachable!(),
    };

    let inner_ret = err!(get_inner_type(inner_ret.clone()).map_err(|e| {
        syn::Error::new_spanned(ret.to_token_stream(), format!("Unexpected return type (should be anyhow::Result<T>).\n{e}"))
    }));

    let struct_name = quote::format_ident!("Endpoint{}", AsPascalCase(name.to_string()).to_string());

    let data = args.clone().input.1;
    let client_type = args.client.clone().map(|c| c.1).unwrap_or(unit());
    let args = args.to_tokens();

    quote::quote! {
        #[doc = concat!("Endpoint Struct for [", stringify!(#name) ,"]\n@ ", stringify!(#method), " -> ", stringify!(#struct_name), "::Data ([", stringify!(#ret), "])")]
        #[derive(Clone)]
        pub struct #struct_name;
        impl milrouter::Endpoint<#client_type> for #struct_name {
            type Data = #data;
            type Returns = #inner_ret;

            fn is_idempotent() -> bool { #idempotent }
        }

        #[cfg(target_arch = "x86_64")]
        impl milrouter::ServerEndpoint<#client_type> for #struct_name {

            fn auth() -> Box<dyn Fn(milrouter::hyper::HeaderMap) -> milrouter::BoxFuture<'static, milrouter::anyhow::Result<#client_type>> + 'static + Send> {
                Box::new(move |i: milrouter::hyper::HeaderMap| Box::pin(#auth(i)))
            }

            fn handler() -> Box<dyn Fn(#client_type, milrouter::hyper::HeaderMap, Self::Data) -> milrouter::BoxFuture<'static, milrouter::anyhow::Result<Self::Returns>> + 'static + Send> {
                Box::new(move |i: #client_type, i2: milrouter::hyper::HeaderMap, i3: Self::Data| Box::pin(#name(i, i2, i3)))
            }
        }


        #[doc("Endpoint Handler for [#name]\n@ #method -> #struct_name::Data ([#arg])")]
        #[cfg(target_arch = "x86_64")]
        pub async fn #name(#args) #ret #block

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

                let error_res = |e, code, label| {
                    milrouter::tracing::info!("[-] {code} {label} /{}", stringify!(#path));
                    milrouter::hyper::Response::builder()
                        .status(code)
                        .body(
                            milrouter::Body::from(format!(
                                "You aren't authorised to access this endpoint\n{e}"
                            ))
                            .full(),
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
                        let bytes = req.collect().await.expect(&format!("Failed to read incoming bytes for {}", stringify!(#inner_name))).to_bytes();
                        std::boxed::Box::new(milrouter::serde_json::from_str::<<#inner as milrouter::Endpoint<_>>::Data>(&String::from_utf8_lossy(&bytes[..]).to_string()).expect(&format!("Failed to deserialize body for {}", stringify!(#inner_name))))
                    }
                };

                let body: <#inner as milrouter::Endpoint<_>>::Data = *body.downcast::<<#inner as milrouter::Endpoint<_>>::Data>().unwrap();
                let handler = <#inner as milrouter::ServerEndpoint<_>>::handler();

                match handler(client, headers, body).await {
                    Ok(response) => {
                        let bytes = milrouter::serde_json::to_vec(&response).expect(&format!("Failed to serialize response for {}", stringify!(#inner_name)));

                        let mut compressed_file = Vec::new();
                        milrouter::gz_compress(bytes.as_slice(), &mut compressed_file).unwrap();

                        milrouter::tracing::info!(concat!("[+] 200 Ok /", stringify!(#path)));
                        return milrouter::hyper::Response::builder()
                            .status(200)
                            .header("Content-Encoding", "gzip")
                            .body(milrouter::Body::from(compressed_file.as_slice()).full())
                            .unwrap();
                    },
                    Err(e) => {
                        milrouter::tracing::warn!(concat!("[-] 400 Bad Request /", stringify!(#path)));
                        return milrouter::hyper::Response::builder()
                            .status(400)
                            .body(milrouter::Body::from(e.to_string()).full())
                            .unwrap()
                    }
                };
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
                        .body(milrouter::Body::from(#html()).full())
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
                                milrouter::Body::from(compressed_file.as_slice()).full()
                            },
                            true => {
                                use std::io::Read;
                                let mut byt = Vec::new();

                                let local = std::fs::File::open(std::path::PathBuf::from(#local_assets).join(&path)).and_then(|mut f| f.read_to_end(&mut byt));
                                let mut compressed_file = Vec::new();
                                milrouter::gz_compress(byt.as_slice(), &mut compressed_file).unwrap();
                                milrouter::Body::from(compressed_file.as_slice()).full()
                            }
                        })
                        .unwrap()
                )
            }
        },
        _ => quote::quote!(),
    };

    let ts = TokenStream::from(quote::quote! {
        #[cfg(target_arch = "x86_64")]
        static __ASSETS: std::sync::LazyLock<std::collections::BTreeMap::<String, (String, &'static [u8])>> = std::sync::LazyLock::new(|| {
            use std::io::Read;
            let mut assets = std::collections::BTreeMap::<String, (String, &'static [u8])>::new();
            #(#inserts)*
            assets
        });

        #[cfg(target_arch = "x86_64")]
        impl #name {
            pub async fn route(req: milrouter::hyper::Request<milrouter::hyper::body::Incoming>) -> std::result::Result<milrouter::hyper::Response<milrouter::http_body_util::Full<milrouter::bytes::Bytes>>, std::convert::Infallible> {
                use milrouter::http_body_util::BodyExt;
                use std::error::Error;

                let path = req.uri().path().to_string();
                let path = path.strip_prefix("/").map(|v| v.to_string()).unwrap_or(path);
                let path = path.strip_prefix("static/").map(|v| v.to_string()).unwrap_or(path);
                let headers = req.headers().clone();

                if req.method() == milrouter::hyper::Method::GET {
                    #assets_serving
                    #default_route_case
                    else {
                        milrouter::tracing::warn!("[#] 404 Not Found /{}", path);
                        return Ok(
                            milrouter::hyper::Response::builder()
                                .status(404)
                                .body(milrouter::Body::default().full())
                                .unwrap()
                        )
                    }
                }

                Ok(match milrouter::tokio::task::spawn(async move {
                    match (path.as_str(), req.method().is_idempotent()) {
                        #(#paths)*
                        path => {
                            milrouter::tracing::info!("[?] 404 Not Found /{}", path.0);
                            return milrouter::hyper::Response::builder()
                                .status(404)
                                .body(milrouter::Body::default().full())
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
                            .unwrap_or("[Unexpected Error]".to_string());

                        milrouter::hyper::Response::builder()
                            .status(500)
                            .body(milrouter::Body::from(format!("{:?}", err)).full())
                            .unwrap()


                    }
                })

            }
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
