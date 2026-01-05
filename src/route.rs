//! Route macro implementation

use proc_macro2::TokenStream;
use quote::quote;
use syn::{FnArg, GenericArgument, ItemFn, PatType, PathArguments, Type, TypePath, parse2};

use crate::parse::RouteInfo;

/// Process the #[route(...)] attribute macro
pub fn route_impl(args: TokenStream, input: TokenStream) -> TokenStream {
    // Parse the route info from args
    let route_info = match RouteInfo::parse(args) {
        Ok(info) => info,
        Err(e) => return e.to_compile_error(),
    };

    // Parse the function
    let func: ItemFn = match parse2(input.clone()) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error(),
    };

    let fn_name = &func.sig.ident;
    let method = route_info.method.to_axum_method();
    let path = &route_info.path;

    // Generate utoipa path annotation with automatic type extraction
    let utoipa_attr = generate_utoipa_attribute(&func, method, &path, &route_info);

    // Store route metadata as a const for the controller to pick up
    let route_const_name = syn::Ident::new(
        &format!("__ROUTE_INFO_{}", fn_name.to_string().to_uppercase()),
        fn_name.span(),
    );

    quote! {
        #utoipa_attr
        #func

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        const #route_const_name: (&'static str, &'static str) = (#method, #path);
    }
}

/// Generate utoipa::path attribute by analyzing function signature
fn generate_utoipa_attribute(
    func: &ItemFn,
    method: &str,
    path: &str,
    route_info: &RouteInfo,
) -> TokenStream {
    let mut request_body_type: Option<TokenStream> = None;
    let mut response_type: Option<TokenStream> = None;
    let mut security_required = false;

    // Analyze arguments to extract request body and security
    for arg in &func.sig.inputs {
        if let FnArg::Typed(PatType { ty, .. }) = arg {
            // Check for Json<T> - request body
            if let Type::Path(TypePath { path, .. }) = &**ty {
                if let Some(segment) = path.segments.last() {
                    if segment.ident == "Json" {
                        if let PathArguments::AngleBracketed(args) = &segment.arguments {
                            if let Some(GenericArgument::Type(Type::Path(TypePath {
                                path, ..
                            }))) = args.args.first()
                            {
                                request_body_type = Some(quote! { #path });
                            }
                        }
                    }
                    // Check for Extension<UserId> - security requirement
                    if segment.ident == "Extension" {
                        if let PathArguments::AngleBracketed(args) = &segment.arguments {
                            if let Some(GenericArgument::Type(Type::Path(TypePath {
                                path, ..
                            }))) = args.args.first()
                            {
                                if path.segments.last().map(|s| s.ident.to_string()).as_deref()
                                    == Some("UserId")
                                {
                                    security_required = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Analyze return type to extract response
    if let syn::ReturnType::Type(_, return_type) = &func.sig.output {
        if let Type::Path(TypePath { path, .. }) = &**return_type {
            if let Some(segment) = path.segments.last() {
                // Handle Result<Json<T>> or ApiResult<Json<T>>
                if segment.ident == "Result" || segment.ident == "ApiResult" {
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(GenericArgument::Type(Type::Path(TypePath { path, .. }))) =
                            args.args.first()
                        {
                            if let Some(inner_segment) = path.segments.last() {
                                if inner_segment.ident == "Json" {
                                    if let PathArguments::AngleBracketed(args) =
                                        &inner_segment.arguments
                                    {
                                        if let Some(GenericArgument::Type(Type::Path(TypePath {
                                            path,
                                            ..
                                        }))) = args.args.first()
                                        {
                                            response_type = Some(quote! { #path });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Generate the utoipa::path attribute
    let method_ident = syn::Ident::new(method, proc_macro2::Span::call_site());
    let summary = route_info.summary.as_deref().unwrap_or("");
    let description = route_info.description.as_deref().unwrap_or("");
    let deprecated = route_info.deprecated;
    // Combine auto-detected security with explicit security flag
    let has_security = security_required || route_info.security;

    // Build the attribute body
    let mut utoipa_body = quote! {
        #method_ident,
        path = #path,
    };

    // Add summary if present
    if !summary.is_empty() {
        utoipa_body = quote! {
            #utoipa_body
            summary = #summary,
        };
    }

    // Add description if present
    if !description.is_empty() {
        utoipa_body = quote! {
            #utoipa_body
            description = #description,
        };
    }

    // Handle tags: tags array > single tag (no fallback here, controller handles final fallback)
    if let Some(ref tags_array) = route_info.tags {
        utoipa_body = quote! {
            #utoipa_body
            tags = [#(#tags_array),*],
        };
    } else if let Some(ref single_tag) = route_info.tag {
        utoipa_body = quote! {
            #utoipa_body
            tag = #single_tag,
        };
    }

    // Add request body if found
    if let Some(body_type) = request_body_type {
        utoipa_body = quote! {
            #utoipa_body
            request_body = #body_type,
        };
    }

    // Add response if found
    if let Some(resp_type) = response_type {
        utoipa_body = quote! {
            #utoipa_body
            responses(
                (status = 200, description = "Success", body = #resp_type),
                (status = 401, description = "Unauthorized"),
                (status = 500, description = "Internal server error")
            ),
        };
    }

    // Add security if UserId extension is present or explicit security flag is set
    if has_security {
        utoipa_body = quote! {
            #utoipa_body
            security(
                ("bearer" = [])
            ),
        };
    }

    // Add deprecated flag
    if deprecated {
        utoipa_body = quote! {
            #utoipa_body
            deprecated,
        };
    }

    quote! {
        #[utoipa::path(
            #utoipa_body
        )]
    }
}
