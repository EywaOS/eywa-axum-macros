//! Parsing utilities for macro attributes.

use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitStr, Path, Token};

/// Parsed controller attributes
#[derive(Debug, FromMeta)]
pub struct ControllerArgs {
    /// API version (e.g., "v1", "v2") - automatically prepended to routes
    #[darling(default)]
    pub version: Option<String>,

    /// URL prefix for all routes (legacy)
    #[darling(default)]
    pub prefix: Option<String>,

    /// URL path prefix for all routes (preferred)
    #[darling(default)]
    pub path: Option<String>,

    /// Application state type
    pub state: Path,

    /// OpenAPI tag name
    #[darling(default)]
    pub tag: Option<String>,

    /// Middleware function to apply (use `middleware = path::to::fn` syntax)
    #[darling(default)]
    pub middleware: Option<Path>,

    /// All routes require bearer authentication (applies to all routes in controller)
    #[darling(default)]
    pub security: bool,

    /// Schema types to register for OpenAPI
    /// usage: schemas(Type1, Type2)
    #[darling(default)]
    pub schemas: PathList,
}

/// Wrapper for a list of paths to support list syntax schemas(A, B)
#[derive(Debug, Default)]
pub struct PathList(pub Vec<Path>);

impl FromMeta for PathList {
    fn from_list(items: &[darling::ast::NestedMeta]) -> darling::Result<Self> {
        let mut paths = Vec::new();
        for item in items {
            if let darling::ast::NestedMeta::Meta(syn::Meta::Path(path)) = item {
                paths.push(path.clone());
            } else {
                return Err(darling::Error::custom("expected path").with_span(item));
            }
        }
        Ok(PathList(paths))
    }
}

/// HTTP method for a route
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
    Trace,
}

impl HttpMethod {
    pub fn from_ident(ident: &Ident) -> Option<Self> {
        let s = ident.to_string().to_uppercase();
        match s.as_str() {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            "PUT" => Some(Self::Put),
            "PATCH" => Some(Self::Patch),
            "DELETE" => Some(Self::Delete),
            "HEAD" => Some(Self::Head),
            "OPTIONS" => Some(Self::Options),
            "TRACE" => Some(Self::Trace),
            _ => None,
        }
    }

    pub fn to_axum_method(&self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Put => "put",
            Self::Patch => "patch",
            Self::Delete => "delete",
            Self::Head => "head",
            Self::Options => "options",
            Self::Trace => "trace",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LinkInfo {
    pub rel: String,
    pub href: String,
    pub method: Option<String>,
}

/// Parsed route information
#[derive(Debug)]
#[allow(dead_code)]
pub struct RouteInfo {
    /// HTTP method
    pub method: HttpMethod,

    /// Route path (e.g., "/:id")
    pub path: String,

    /// OpenAPI summary
    pub summary: Option<String>,

    /// OpenAPI description
    pub description: Option<String>,

    /// Whether this route is deprecated
    pub deprecated: bool,

    /// Whether this route requires authentication (Extension<UserId>)
    pub security: bool,

    /// Wrap in CollectionResponse
    pub collection: bool,

    /// Wrap in HateoasResponse
    pub hateoas: bool,

    /// Single tag (legacy: `tag = "Timer"`)
    pub tag: Option<String>,

    /// Multiple tags (new: `tags = ["Timer", "Admin"]`)
    pub tags: Option<Vec<String>>,

    /// Additional attributes to pass to utoipa (e.g., responses(...))
    pub other_attrs: Vec<(Ident, TokenStream)>,

    /// HATEOAS links for the response wrapping
    /// usage: links( (rel="self", href="/..."), ... )
    pub links: Vec<LinkInfo>,

    /// Raw content of responses(...) attribute, enabling merging
    pub responses: Option<TokenStream>,
}

impl RouteInfo {
    /// Parse route attributes from tokens
    pub fn parse(tokens: TokenStream) -> syn::Result<Self> {
        struct RouteAttr {
            method: HttpMethod,
            path: String,
            summary: Option<String>,
            description: Option<String>,
            deprecated: bool,
            security: bool,
            collection: bool,
            hateoas: bool,
            tag: Option<String>,
            tags: Option<Vec<String>>,
            other_attrs: Vec<(Ident, TokenStream)>,
            links: Vec<LinkInfo>,
            responses: Option<TokenStream>,
        }

        impl Parse for RouteAttr {
            fn parse(input: ParseStream) -> syn::Result<Self> {
                // Parse METHOD
                let method_ident: Ident = input.parse()?;
                let method = HttpMethod::from_ident(&method_ident)
                    .ok_or_else(|| syn::Error::new_spanned(&method_ident, "Invalid HTTP method"))?;

                // Parse path string
                let path_lit: LitStr = input.parse()?;
                let path = path_lit.value();

                let mut summary = None;
                let mut description = None;
                let mut deprecated = false;
                let mut collection = false;
                let mut hateoas = false;
                let mut security = false;
                let mut tag: Option<String> = None;
                let mut tags: Option<Vec<String>> = None;
                let mut other_attrs = Vec::new();
                let mut links = Vec::new();
                let mut responses = None;

                // Parse optional key=value pairs
                while input.peek(Token![,]) {
                    let _: Token![,] = input.parse()?;

                    if input.is_empty() {
                        break;
                    }

                    let key: Ident = input.parse()?;
                    let key_str = key.to_string();

                    match key_str.as_str() {
                        "summary" => {
                            let _: Token![=] = input.parse()?;
                            let val: LitStr = input.parse()?;
                            summary = Some(val.value());
                        }
                        "description" => {
                            let _: Token![=] = input.parse()?;
                            let val: LitStr = input.parse()?;
                            description = Some(val.value());
                        }
                        "deprecated" => {
                            deprecated = true;
                        }
                        "security" => {
                            security = true;
                        }
                        "collection" => {
                            collection = true;
                        }
                        "hateoas" => {
                            hateoas = true;
                        }
                        "tag" => {
                            let _: Token![=] = input.parse()?;
                            let val: LitStr = input.parse()?;
                            tag = Some(val.value());
                        }
                        "tags" => {
                            let _: Token![=] = input.parse()?;
                            let content;
                            syn::bracketed!(content in input);
                            let mut tag_list = Vec::new();
                            while !content.is_empty() {
                                let tag_val: LitStr = content.parse()?;
                                tag_list.push(tag_val.value());
                                if !content.is_empty() {
                                    let _: Token![,] = content.parse()?;
                                }
                            }
                            tags = Some(tag_list);
                        }
                        "links" => {
                            let content;
                            syn::parenthesized!(content in input);
                            let mut link_list = Vec::new();
                            while !content.is_empty() {
                                let inner;
                                syn::parenthesized!(inner in content);
                                let mut rel = String::new();
                                let mut href = String::new();
                                let mut method = None;

                                while !inner.is_empty() {
                                    let key: Ident = inner.parse()?;
                                    let _: Token![=] = inner.parse()?;
                                    let val: LitStr = inner.parse()?;
                                    match key.to_string().as_str() {
                                        "rel" => rel = val.value(),
                                        "href" => href = val.value(),
                                        "method" => method = Some(val.value()),
                                        _ => {}
                                    }
                                    if !inner.is_empty() {
                                        let _: Token![,] = inner.parse()?;
                                    }
                                }
                                link_list.push(LinkInfo { rel, href, method });
                                if !content.is_empty() {
                                    let _: Token![,] = content.parse()?;
                                }
                            }
                            links = link_list;
                        }
                        "responses" => {
                            let content;
                            syn::parenthesized!(content in input);
                            let val: TokenStream = content.parse()?;
                            responses = Some(val);
                        }
                        _ => {
                            // Capture any other attribute (like responses)
                            if input.peek(Token![=]) {
                                let _: Token![=] = input.parse()?;
                                // Parse until next comma or end
                                // This is tricky because the value might contain commas (e.g. tuples)
                                // Standard way: parse as Expr
                                let val: Expr = input.parse()?;
                                other_attrs.push((key, val.to_token_stream()));
                            } else if input.peek(syn::token::Paren) {
                                // Capture parenthesized content e.g. responses(...)
                                let content;
                                syn::parenthesized!(content in input);
                                let val: TokenStream = content.parse()?;
                                // Wrap back in parens for the macro output
                                let quoted = quote::quote! { (#val) };
                                other_attrs.push((key, quoted));
                            } else {
                                // Boolean flag
                                other_attrs.push((key, quote::quote! {}));
                            }
                        }
                    }
                }

                Ok(RouteAttr {
                    method,
                    path,
                    summary,
                    description,
                    deprecated,
                    security,
                    collection,
                    hateoas,
                    tag,
                    tags,
                    other_attrs,
                    links,
                    responses,
                })
            }
        }

        let attr: RouteAttr = syn::parse2(tokens)?;

        Ok(RouteInfo {
            method: attr.method,
            path: attr.path,
            summary: attr.summary,
            description: attr.description,
            deprecated: attr.deprecated,
            security: attr.security,
            collection: attr.collection,
            hateoas: attr.hateoas,
            tag: attr.tag,
            tags: attr.tags,
            other_attrs: attr.other_attrs,
            links: attr.links,
            responses: attr.responses,
        })
    }
}
