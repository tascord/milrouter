#[macro_use]
mod helpers;
mod macro_impl;

#[proc_macro_attribute]
pub fn endpoint(annot: TokenStream, item: TokenStream) -> TokenStream {
    macro_impl::endpoint::expand_endpoint(annot, item)
}

#[proc_macro_derive(Router, attributes(assets, html, middleware))]
pub fn router(item: TokenStream) -> TokenStream {
    macro_impl::router::expand_router(item)
}

#[proc_macro_attribute]
pub fn assets(_: TokenStream, i: TokenStream) -> TokenStream { i }

#[proc_macro_attribute]
pub fn html(_: TokenStream, i: TokenStream) -> TokenStream { i }

#[proc_macro_attribute]
pub fn middleware(_: TokenStream, i: TokenStream) -> TokenStream { i }

use proc_macro::TokenStream;
