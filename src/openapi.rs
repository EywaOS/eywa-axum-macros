//! OpenAPI macro implementation
//!
//! This module provides the `openapi_for!` procedural macro
//! that generates OpenAPI documentation structs automatically.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Path, Token, bracketed, punctuated::Punctuated};

/// Arguments for the openapi_for! macro
pub struct OpenApiForArgs {
    /// List of controllers to include
    pub controllers: Vec<Path>,
    /// Additional schemas to include
    pub schemas: Vec<Path>,
    /// Tags definitions
    pub tags: Vec<TagDef>,
    /// API info
    pub info: Option<ApiInfo>,
}

/// Tag definition for OpenAPI
pub struct TagDef {
    pub name: String,
    pub description: String,
}

/// API Info for OpenAPI
pub struct ApiInfo {
    pub title: String,
    pub version: String,
    pub description: Option<String>,
}

impl Parse for OpenApiForArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut controllers = Vec::new();
        let mut schemas = Vec::new();
        let mut tags = Vec::new();
        let mut info = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let key_str = key.to_string();

            match key_str.as_str() {
                "controllers" => {
                    let _: Token![=] = input.parse()?;
                    let content;
                    bracketed!(content in input);
                    let paths: Punctuated<Path, Token![,]> =
                        content.parse_terminated(Path::parse, Token![,])?;
                    controllers = paths.into_iter().collect();
                }
                "schemas" => {
                    let _: Token![=] = input.parse()?;
                    let content;
                    bracketed!(content in input);
                    let paths: Punctuated<Path, Token![,]> =
                        content.parse_terminated(Path::parse, Token![,])?;
                    schemas = paths.into_iter().collect();
                }
                "tags" => {
                    let _: Token![=] = input.parse()?;
                    let content;
                    bracketed!(content in input);
                    // Parse tags as: (name = "...", description = "...")
                    while !content.is_empty() {
                        let tag_content;
                        syn::parenthesized!(tag_content in content);

                        let mut name = String::new();
                        let mut description = String::new();

                        while !tag_content.is_empty() {
                            let field_key: Ident = tag_content.parse()?;
                            let _: Token![=] = tag_content.parse()?;
                            let val: LitStr = tag_content.parse()?;

                            match field_key.to_string().as_str() {
                                "name" => name = val.value(),
                                "description" => description = val.value(),
                                _ => {}
                            }

                            if !tag_content.is_empty() {
                                let _: Token![,] = tag_content.parse()?;
                            }
                        }

                        tags.push(TagDef { name, description });

                        if !content.is_empty() {
                            let _: Token![,] = content.parse()?;
                        }
                    }
                }
                "info" => {
                    let _: Token![=] = input.parse()?;
                    let info_content;
                    syn::parenthesized!(info_content in input);

                    let mut title = String::new();
                    let mut version = String::new();
                    let mut description = None;

                    while !info_content.is_empty() {
                        let field_key: Ident = info_content.parse()?;
                        let _: Token![=] = info_content.parse()?;
                        let val: LitStr = info_content.parse()?;

                        match field_key.to_string().as_str() {
                            "title" => title = val.value(),
                            "version" => version = val.value(),
                            "description" => description = Some(val.value()),
                            _ => {}
                        }

                        if !info_content.is_empty() {
                            let _: Token![,] = info_content.parse()?;
                        }
                    }

                    info = Some(ApiInfo {
                        title,
                        version,
                        description,
                    });
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        key,
                        format!("Unknown argument: {}", key_str),
                    ));
                }
            }

            // Consume trailing comma if present
            if input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
            }
        }

        Ok(OpenApiForArgs {
            controllers,
            schemas,
            tags,
            info,
        })
    }
}

/// Implementation of the openapi_for! macro
pub fn openapi_for_impl(input: TokenStream) -> TokenStream {
    let args: OpenApiForArgs = match syn::parse2(input) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error(),
    };

    // Generate paths from controllers
    // Each controller should expose a __UTOIPA_PATHS__ module
    let controller_paths: Vec<TokenStream> = args
        .controllers
        .iter()
        .map(|controller| {
            // Convert controller path to its __UTOIPA_PATHS__ module
            // e.g., timer_controller::__UTOIPA_PATHS__::*
            quote! {
                #controller::__UTOIPA_PATHS__
            }
        })
        .collect();

    // Generate schema list
    let schema_list: Vec<&Path> = args.schemas.iter().collect();

    // Generate tags
    let tags_tokens: Vec<TokenStream> = args
        .tags
        .iter()
        .map(|tag| {
            let name = &tag.name;
            let description = &tag.description;
            quote! {
                (name = #name, description = #description)
            }
        })
        .collect();

    // Generate info section if provided
    let info_tokens = if let Some(info) = &args.info {
        let title = &info.title;
        let version = &info.version;
        let desc = info.description.as_deref().unwrap_or("");
        if desc.is_empty() {
            quote! {
                info(title = #title, version = #version),
            }
        } else {
            quote! {
                info(title = #title, version = #version, description = #desc),
            }
        }
    } else {
        quote! {}
    };

    // Generate the paths list
    // We need to collect all path functions from each controller
    let paths_tokens = if controller_paths.is_empty() {
        quote! {}
    } else {
        quote! {
            paths(
                // Note: Individual paths need to be listed manually or
                // controllers need to expose a list.
                // This is a limitation of proc macros - they can't "see" compiled code.
            ),
        }
    };

    // Generate schemas
    let schemas_tokens = if schema_list.is_empty() {
        quote! {}
    } else {
        quote! {
            components(
                schemas(#(#schema_list),*)
            ),
        }
    };

    // Generate tags
    let tags_section = if tags_tokens.is_empty() {
        quote! {}
    } else {
        quote! {
            tags(
                #(#tags_tokens),*
            ),
        }
    };

    quote! {
        #[derive(utoipa::OpenApi)]
        #[openapi(
            #info_tokens
            #paths_tokens
            #schemas_tokens
            #tags_section
        )]
        pub struct ApiDoc;
    }
}
