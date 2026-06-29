use {
    heck::AsPascalCase,
    crate::helpers::{RouteInfo, get_inner_type, parse_fn_args, unit},
    proc_macro::TokenStream,
    quote::{ToTokens, quote},
    syn::{FnArg, parse_macro_input},
};

pub fn expand_endpoint(annot: TokenStream, item: TokenStream) -> TokenStream {
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

    let client_endpoint_impl = if is_stream {
        quote! {}
    } else if is_raw {
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

    let server_endpoint_impl = if is_stream {
        quote! {
            #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
            impl milrouter::ServerEndpoint<#client_type> for #struct_name {
                fn auth() -> milrouter::AsyncHandler<milrouter::hyper::HeaderMap, milrouter::anyhow::Result<#client_type>> {
                    Box::new(move |i: milrouter::hyper::HeaderMap| Box::pin(#auth(i)))
                }

                fn handler() -> milrouter::AsyncHandler3<#client_type, milrouter::hyper::HeaderMap, Self::Data, milrouter::anyhow::Result<Self::Returns>> {
                    Box::new(move |_, _, _| Box::pin(async { unreachable!("Internal error: handler() should not be called for streaming endpoints; use stream_handler() instead.") }))
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