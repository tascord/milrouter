use {
    proc_macro2::{Punct, TokenStream, TokenTree},
    quote::{ToTokens, format_ident, quote},
    std::{collections::HashMap, env, str::FromStr},
    syn::{Data, DataEnum, DeriveInput, Ident, Type},
};

macro_rules! err {
    ($result:expr) => {
        match $result {
            Err(e) => return e.into_compile_error().into(),
            Ok(e) => e,
        }
    };
}

pub fn unit() -> Type { syn::parse_str("()").unwrap() }

pub fn preamble(input: DeriveInput) -> (DeriveInput, Ident, DataEnum) {
    let name = input.clone().ident;
    let data = match input.clone().data {
        Data::Enum(data) => data,
        _ => panic!("Router can only be implemented for enums"),
    };

    (input, name, data)
}

pub fn get_inner_type(t: Type) -> Result<Type, syn::Error> {
    match t {
        Type::Path(ref p) => {
            let ty = match p.path.segments.last().unwrap().arguments.clone() {
                syn::PathArguments::AngleBracketed(t) => t,
                _ => return Err(syn::Error::new_spanned(p.to_token_stream(), "Unexpected path arguments in type")),
            }
            .args;

            let ty = match ty.first().unwrap() {
                syn::GenericArgument::Type(t) => t,
                _ => {
                    return Err(syn::Error::new_spanned(
                        p.to_token_stream(),
                        "Unexpected non-type generic argument in type",
                    ));
                }
            };

            Ok(ty.clone())
        }
        _ => Err(syn::Error::new_spanned(t.to_token_stream(), "Cant get inner type of non-path")),
    }
}

pub fn type_contains(t: Type, s: String) -> bool {
    match t {
        Type::Path(ref p) => p.path.segments.iter().any(|p| match p.arguments.clone() {
            syn::PathArguments::AngleBracketed(t) => t.args.iter().any(|t| match t {
                syn::GenericArgument::Type(t) => type_contains(t.clone(), s.to_string()),
                _ => false,
            }),
            _ => false,
        }),
        Type::Verbatim(t) => t.to_string().to_lowercase().contains(&s),
        _ => false,
    }
}

#[allow(unused_variables)]
#[derive(Debug)]
pub struct RouteInfo {
    pub is_idempotent: bool,
    pub auth: proc_macro2::TokenStream,
}

impl RouteInfo {
    fn parse_groups(
        map: &mut HashMap<String, (String, proc_macro2::TokenStream)>,
        buf: &mut Vec<String>,
        tbuf: &mut proc_macro2::TokenStream,
    ) -> Result<(), syn::Error> {
        match buf.clone().len() {
            0 => {}
            1 => {
                map.insert(buf.first().unwrap().to_string(), (true.to_string(), tbuf.clone()));
            }
            2 => {
                map.insert(buf.first().unwrap().to_string(), (buf.last().unwrap().to_string(), tbuf.clone()));
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    tbuf.clone(),
                    "Attributes should either have a single value (idempotent = true), or be present to indicate a value \
                     of 'true'.",
                ));
            }
        };

        buf.clear();
        *tbuf = proc_macro2::TokenStream::new();

        Ok(())
    }

    fn push_or_append(s: &str, buf: &mut Vec<String>) {
        match buf.last().cloned().unwrap_or_default().ends_with(['(', ')', ':']) || s.starts_with(['(', ')', ':']) {
            true => buf.last_mut().unwrap().push_str(s),
            false => buf.push(s.to_string()),
        }
    }

    pub fn parse(tokens: proc_macro2::TokenStream) -> Result<Self, syn::Error> {
        let mut map = HashMap::<String, (String, proc_macro2::TokenStream)>::new();
        let mut buf = Vec::<String>::new();
        let mut tbuf = proc_macro2::TokenStream::new();

        for token in tokens.clone().into_iter() {
            tbuf.extend(token.to_token_stream());
            match token.clone() {
                proc_macro2::TokenTree::Ident(i) => RouteInfo::push_or_append(&i.to_string(), &mut buf),
                proc_macro2::TokenTree::Literal(l) => RouteInfo::push_or_append(
                    l.to_string().strip_prefix("\"").and_then(|s| s.strip_suffix("\"")).unwrap_or(&l.to_string()),
                    &mut buf,
                ),
                proc_macro2::TokenTree::Punct(p) => match p.as_char() {
                    '=' => {}
                    '(' | ')' | ':' => RouteInfo::push_or_append(&p.to_string(), &mut buf),
                    ',' => RouteInfo::parse_groups(&mut map, &mut buf, &mut tbuf)?,

                    el => return Err(syn::Error::new_spanned(token, format!("Unexpected punctuation mark: {el}"))),
                },
                _ => return Err(syn::Error::new_spanned(token, "I have no idea what this guy is doing here")),
            }
        }

        RouteInfo::parse_groups(&mut map, &mut buf, &mut tbuf)?;

        Ok(RouteInfo {
            is_idempotent: {
                let (v, t) = map.get("idempotent").cloned().unwrap_or((false.to_string(), Default::default()));

                v.parse::<bool>()
                    .map_err(|_| syn::Error::new_spanned(t, "Attribute 'idempotent' must be a valid boolean"))?
            },

            auth: {
                map.get("auth")
                    .cloned()
                    .map(|a| proc_macro2::TokenStream::from_str(&a.0).unwrap())
                    .ok_or(syn::Error::new_spanned(tokens, "No auth handler provided"))?
            },
        })
    }
}

#[derive(Clone)]
pub struct PartialFnArgs {
    pub client: Option<(Ident, Type)>,
    pub input: (Ident, Type),
    pub headers: Option<Ident>,
}

impl PartialFnArgs {
    pub fn to_tokens(&self) -> TokenStream {
        let mut ts = TokenStream::new();

        let mut push = |t: TokenStream| {
            if !ts.is_empty() {
                let c = TokenTree::Punct(Punct::new(',', proc_macro2::Spacing::Alone));
                ts.extend(c.to_token_stream());
            }
            ts.extend(t.to_token_stream());
        };

        if let Some((i, t)) = self.client.clone() {
            push(quote!(#i: #t));
        } else {
            let i = format_ident!("_");
            let t = unit();
            push(quote!(#i: #t));
        }

        if let Some(i) = self.headers.clone() {
            push(quote!(#i: milrouter::hyper::HeaderMap));
        } else {
            let i = format_ident!("_");
            push(quote!(#i: milrouter::hyper::HeaderMap));
        }

        let (i, t) = self.input.clone();
        push(quote!(#i: #t));

        ts
    }
}

impl Default for PartialFnArgs {
    fn default() -> Self { Self { client: None, input: (format_ident!("_"), unit()), headers: None } }
}

pub fn parse_fn_args(a: Vec<(Ident, Type)>) -> PartialFnArgs {
    a.iter().fold(PartialFnArgs::default(), |mut a, b| {
        let is_client_ty =
            type_contains(b.1.clone(), "client".to_string()) || b.0.to_string().to_lowercase().contains("client");

        if is_client_ty {
            a.client.replace(b.clone());
        } else if b.0.to_string().to_lowercase().contains("header") {
            a.headers.replace(b.0.clone());
        } else if a.input.0 != "_" {
            panic!("Unexpected non-client argument {}, as input is already defined as {}", b.0, a.input.1.to_token_stream())
        } else {
            a.input = b.clone()
        }

        a
    })
}

fn strip(a: &str) -> String {
    a.strip_prefix("\"").and_then(|b| b.strip_suffix("\"")).map(|v| v.to_string()).unwrap_or(a.to_string())
}

pub fn parse_attrs(input: DeriveInput) -> (Option<TokenStream>, Option<String>) {
    let local_assets = input.attrs.iter().find(|a| a.path().is_ident("assets"));
    let local_assets = local_assets.map(|a| {
        err!(
            a.parse_args::<syn::LitStr>()
                .map(|a| a.to_token_stream())
                .map_err(|_| syn::Error::new_spanned(a.into_token_stream(), "Assets attribute should be a literal string"))
        )
    });

    let html = input.attrs.iter().find(|a| a.path().is_ident("html"));
    let html = html.map(|a| {
        err!(
            a.parse_args::<syn::Expr>()
                .map(|a| a.to_token_stream())
                .map_err(|_| syn::Error::new_spanned(a.into_token_stream(), "HTML attribute should point to a function"))
        )
    });

    let local_assets = local_assets.map(|a| {
        format!("{}/{}", env::current_dir().map(|d| d.display().to_string()).unwrap_or_default(), strip(&a.to_string()))
    });

    (html, local_assets)
}
