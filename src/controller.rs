//! Controller macro implementation

use darling::FromMeta;
use darling::ast::NestedMeta;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, ImplItem, ItemImpl, parse2};

use crate::parse::{ControllerArgs, RouteInfo};

/// Process the #[controller(...)] attribute macro
pub fn controller_impl(args: TokenStream, input: TokenStream) -> TokenStream {
    // Parse controller args
    let meta_list = match NestedMeta::parse_meta_list(args) {
        Ok(m) => m,
        Err(e) => return e.to_compile_error(),
    };

    let controller_args = match ControllerArgs::from_list(&meta_list) {
        Ok(a) => a,
        Err(e) => return e.write_errors(),
    };

    // Parse the impl block
    let mut impl_block: ItemImpl = match parse2(input) {
        Ok(i) => i,
        Err(e) => return e.to_compile_error(),
    };

    // Extract the controller type name
    let self_ty = &impl_block.self_ty;
    let state_ty = &controller_args.state;

    // Build version prefix (e.g., "/v1")
    let version_prefix = if let Some(ref version) = controller_args.version {
        format!("/{}", version)
    } else {
        String::new()
    };

    // Prefer path, fallback to prefix (legacy), default to empty
    let path_prefix = controller_args
        .path
        .or(controller_args.prefix)
        .unwrap_or_default();

    // Combine version and path prefix (e.g., "/v1/projects")
    let full_prefix = format!("{}{}", version_prefix, path_prefix);

    let tag = controller_args.tag.clone().unwrap_or_else(|| {
        // Extract name from type
        if let syn::Type::Path(tp) = self_ty.as_ref() {
            tp.path
                .segments
                .last()
                .map(|s| s.ident.to_string().replace("Controller", ""))
                .unwrap_or_else(|| "API".to_string())
        } else {
            "API".to_string()
        }
    });

    // Controller-level security - applies to all routes
    let controller_security = controller_args.security;

    // Schema types to register
    let schema_types = &controller_args.schemas.0;

    // Phase 1: HATEOAS Transformation
    let mut new_items = Vec::new();
    let original_items: Vec<_> = impl_block.items.drain(..).collect();

    for item in original_items {
        if let syn::ImplItem::Fn(mut method) = item {
            // Find route attr
            let route_attr_idx = method.attrs.iter().position(|a| a.path().is_ident("route"));

            let mut links = Vec::new();
            if let Some(idx) = route_attr_idx {
                if let Ok(info) = parse_route_attr(&method.attrs[idx]) {
                    if !info.links.is_empty() {
                        links = info.links;
                    }
                }
            }

            if !links.is_empty() {
                let original_ident = method.sig.ident.clone();
                let impl_ident = quote::format_ident!("__impl_{}", original_ident);

                // 1. Implementation Method (renamed, hidden, no route attr)
                let mut impl_method = method.clone();
                impl_method.sig.ident = impl_ident.clone();
                impl_method.attrs.retain(|a| !a.path().is_ident("route"));
                impl_method.attrs.push(syn::parse_quote!(#[doc(hidden)]));
                impl_method
                    .attrs
                    .push(syn::parse_quote!(#[allow(non_snake_case)]));

                new_items.push(syn::ImplItem::Fn(impl_method));

                // 2. Wrapper Method (original name, wrapped logic)
                let mut wrapper_method = method.clone();

                // Extract return type inner T
                // Extract return type inner T
                let inner_type_opt =
                    if let syn::ReturnType::Type(_, ty) = &wrapper_method.sig.output {
                        extract_inner_type(ty)
                    } else {
                        None
                    };

                if let Some(inner_type) = inner_type_opt {
                    // Change return type to Result<Json<HateoasResponse<T>>>
                    // Use short names assuming they are in scope (via prelude) to help Utoipa resolution
                    let new_output: syn::ReturnType = syn::parse_quote! {
                        -> Result<Json<HateoasResponse<#inner_type>>>
                    };
                    wrapper_method.sig.output = new_output;

                    // forward args
                    let args: Vec<_> = wrapper_method
                        .sig
                        .inputs
                        .iter()
                        .flat_map(|arg| match arg {
                            syn::FnArg::Typed(pat) => collect_pat_idents(&pat.pat),
                            _ => Vec::new(),
                        })
                        .collect();

                    // Links statements
                    let link_stmts = links.iter().map(|l| {
                        let rel = &l.rel;
                        let href = &l.href;
                        let method = l.method.as_deref().unwrap_or("GET");
                        quote! {
                            h = h.add_link(#rel, Link::new(#href).method(#method));
                        }
                    });

                    let wrapper_body = quote! {
                        {
                            let resp = Self::#impl_ident( #(#args),* ).await?;
                            let Json(data) = resp;
                            let mut h = HateoasResponse::new(data);
                            #(#link_stmts)*
                            Ok(Json(h))
                        }
                    };
                    wrapper_method.block = syn::parse2(wrapper_body).expect("Invalid wrapper body");
                    new_items.push(syn::ImplItem::Fn(wrapper_method));
                } else {
                    new_items.push(syn::ImplItem::Fn(method));
                }
            } else {
                new_items.push(syn::ImplItem::Fn(method));
            }
        } else {
            new_items.push(item);
        }
    }
    impl_block.items = new_items;

    // Collect route information from methods
    let mut routes = Vec::new();

    for item in &mut impl_block.items {
        if let ImplItem::Fn(method) = item {
            // Look for #[route(...)] attribute
            let route_attr_idx = method.attrs.iter().position(|a| a.path().is_ident("route"));

            if let Some(idx) = route_attr_idx {
                let attr = method.attrs.remove(idx);

                // Parse route info
                if let Ok(route_info) = parse_route_attr(&attr) {
                    let fn_name = &method.sig.ident;
                    routes.push((fn_name.clone(), route_info, method.sig.clone()));
                }
            }
        }
    }

    // Generate route registrations
    let route_registrations: Vec<_> = routes
        .iter()
        .map(|(fn_name, route_info, _)| {
            let method = format_ident!("{}", route_info.method.to_axum_method());
            let path = &route_info.path;

            quote! {
                .route(#path, eywa_axum::axum::routing::#method(Self::#fn_name))
            }
        })
        .collect();

    // Generate middleware layers
    let middleware_layers: Vec<_> = controller_args
        .middleware
        .iter()
        .map(|m| {
            quote! {
                .layer(eywa_axum::axum::middleware::from_fn_with_state(state.clone(), #m))
            }
        })
        .collect();

    // Generate utoipa wrapper functions
    let utoipa_wrappers: Vec<_> = routes
        .iter()
        .map(|(fn_name, route_info, method_sig)| {
            let full_path = format!("{}{}", full_prefix, route_info.path);
            let method_ident = syn::Ident::new(
                route_info.method.to_axum_method(),
                proc_macro2::Span::call_site(),
            );
            let summary = route_info.summary.as_deref().unwrap_or("");
            // Append HATEOAS links to description
            let mut desc_string = route_info.description.as_deref().unwrap_or("").to_string();
            if !route_info.links.is_empty() {
                if !desc_string.is_empty() {
                    desc_string.push_str("\n\n");
                }
                desc_string.push_str("**Available Links:**\n");
                for link in &route_info.links {
                    let method = link.method.as_deref().unwrap_or("GET");
                    desc_string
                        .push_str(&format!("- `{}`: `{} {}`\n", link.rel, method, link.href));
                }
            }
            let description = desc_string.as_str();
            let deprecated = route_info.deprecated;

            // Build utoipa::path attribute body
            let mut utoipa_body = quote! {
                #method_ident,
                path = #full_path,
            };

            if !summary.is_empty() {
                utoipa_body = quote! {
                    #utoipa_body
                    summary = #summary,
                };
            }

            if !description.is_empty() {
                utoipa_body = quote! {
                    #utoipa_body
                    description = #description,
                };
            }

            // Handle tags with priority: route tags array > route single tag > controller tag
            if let Some(ref tags_array) = route_info.tags {
                // Multiple tags from route
                utoipa_body = quote! {
                    #utoipa_body
                    tags = [#(#tags_array),*],
                };
            } else if let Some(ref single_tag) = route_info.tag {
                // Single tag from route (legacy support)
                utoipa_body = quote! {
                    #utoipa_body
                    tag = #single_tag,
                };
            } else {
                // Fallback to controller tag
                utoipa_body = quote! {
                    #utoipa_body
                    tag = #tag,
                };
            }

            if deprecated {
                utoipa_body = quote! {
                    #utoipa_body
                    deprecated,
                };
            }

            // Add security if specified at route OR controller level
            // Route security takes precedence, but if controller has security, all routes get it
            let needs_security = route_info.security || controller_security;
            if needs_security {
                utoipa_body = quote! {
                    #utoipa_body
                    security(("bearer" = [])),
                };
            }

            // Inject other attributes (like responses(...), params(...))
            let other_tokens = route_info.other_attrs.iter().map(|(id, toks)| {
                quote! { #id #toks, }
            });

            utoipa_body = quote! {
                #utoipa_body
                #(#other_tokens)*
            };

            let mut extra_structs = quote! {};
            let mut success_body_type = quote! {};
            let mut override_stub_output: Option<syn::ReturnType> = None;

            // Check if generic HATEOAS response
            let auto_success = if let syn::ReturnType::Type(_, ty) = &method_sig.output {
                 if let Some(inner) = extract_inner_type(ty) {
                     // Check if inner is HateoasResponse<T>
                     if let Some(hateoas_inner) = extract_hateoas_inner_type(&inner) {
                         // Generate concrete struct for Utoipa
                         let struct_name = quote::format_ident!("__HateoasSchema_{}", fn_name);
                         extra_structs = quote! {
                             #[derive(eywa_axum::Serialize, eywa_axum::Deserialize, eywa_axum::utoipa::ToSchema)]
                             #[allow(non_camel_case_types)]
                             pub struct #struct_name {
                                 pub data: #hateoas_inner,
                                 pub links: std::collections::HashMap<String, eywa_axum::Link>,
                             }
                         };
                         success_body_type = quote! { #struct_name };
                         override_stub_output = Some(syn::parse_quote! { -> eywa_axum::Json<#struct_name> });
                         quote! { (status = 200, body = #struct_name), }
                     } else {
                         // Standard response
                         quote! { (status = 200, body = #inner), }
                     }
                 } else {
                     quote! {}
                 }
            } else {
                 quote! {}
            };
            
            // Override auto_success if user provided 200 manually... (logic below)
            let user_resp = &route_info.responses;
            let user_token_str = user_resp.as_ref().map(|t| t.to_string()).unwrap_or_default();
            
            let final_success = if !user_token_str.contains("200") && !user_token_str.contains("OK") {
                auto_success
            } else {
                quote! {}
            };

            let auto_401 =
                if !user_token_str.contains("401") && !user_token_str.contains("Unauthorized") {
                    quote! { (status = 401, description = "Unauthorized"), }
                } else {
                    quote! {}
                };

            let auto_500 = if !user_token_str.contains("500")
                && !user_token_str.contains("Internal server error")
            {
                quote! { (status = 500, description = "Internal server error"), }
            } else {
                quote! {}
            };

            let combined_responses = if let Some(tokens) = user_resp {
                 quote! { #tokens, #final_success #auto_401 #auto_500 }
            } else {
                 quote! { #final_success #auto_401 #auto_500 }
            };

            utoipa_body = quote! {
               #utoipa_body
               responses(
                   #combined_responses
               ),
            };

            // Use original function signature for stub to allow Utoipa auto-discovery
            // Filter out 'self'
            let stub_inputs = method_sig.inputs.iter().filter(|arg| match arg {
                syn::FnArg::Receiver(_) => false,
                _ => true,
            });
            let stub_output = override_stub_output.as_ref().unwrap_or(&method_sig.output);

            quote! {
                #[utoipa::path(
                    #utoipa_body
                )]
                #[allow(dead_code, unused_variables)]
                pub async fn #fn_name(
                    #(#stub_inputs),*
                ) #stub_output {
                    unreachable!("This is a stub for utoipa - use controller method instead");
                }
                
                #extra_structs
            }
        })
        .collect();

    // Generate OpenAPI paths for utoipa
    let openapi_paths: Vec<_> = routes
        .iter()
        .map(|(_fn_name, route_info, _method_sig)| {
            let full_path = format!("{}{}", full_prefix, route_info.path);
            let method_str = route_info.method.to_axum_method().to_uppercase();
            let summary = route_info.summary.as_deref().unwrap_or("");
            let description = route_info.description.as_deref().unwrap_or("");
            let tag = &tag;

            quote! {
                eywa_axum::OpenApiPath {
                    path: #full_path.to_string(),
                    method: #method_str.to_string(),
                    summary: #summary.to_string(),
                    description: #description.to_string(),
                    tag: #tag.to_string(),
                }
            }
        })
        .collect();

    // Prepare generated struct names for register_paths
    // Utoipa generates structs like __path_functionName
    let path_structs: Vec<_> = routes
        .iter()
        .map(|(ident, _, _)| quote::format_ident!("__path_{}", ident))
        .collect();

    // Generate the into_router implementation
    let into_router_impl = quote! {
        impl eywa_axum::IntoRouter<#state_ty> for #self_ty {
            /// Creates an axum Router from this controller.
            ///
            /// The router includes all routes defined with `#[route(...)]`.
            fn into_router(state: #state_ty) -> eywa_axum::axum::Router<#state_ty> {
                eywa_axum::axum::Router::new()
                    #(#route_registrations)*
                    #(#middleware_layers)*
                    .with_state(state)
            }

            /// Returns the URL prefix for this controller.
            /// Includes version prefix if specified (e.g., "/v1").
            fn prefix() -> &'static str {
                #full_prefix
            }

            /// Returns the OpenAPI tag for this controller.
            fn tag() -> &'static str {
                #tag
            }

            /// Returns route metadata for OpenAPI generation.
            fn openapi_routes() -> Vec<eywa_axum::OpenApiPath> {
                vec![
                    #(#openapi_paths),*
                ]
            }

            /// Register schemas used by this controller.
            fn register_schemas(components: &mut utoipa::openapi::Components) {
                #(
                    {
                        use utoipa::{ToSchema, PartialSchema};
                        let name = <#schema_types as ToSchema>::name().to_string();
                        let schema = <#schema_types as PartialSchema>::schema();
                        components.schemas.insert(name, schema);
                    }
                )*
            }

            /// Register paths in the OpenAPI spec.
            fn register_paths(openapi: &mut utoipa::openapi::OpenApi) {
                #(
                    {
                        // Utoipa generates a struct __path_FnName for each path
                        use __UTOIPA_PATHS__::*;

                        // Use pre-calculated struct name and fully qualified Path trait calls
                        let path = <#path_structs as utoipa::Path>::path();
                        let methods = <#path_structs as utoipa::Path>::methods();
                        let mut operation = <#path_structs as utoipa::Path>::operation();

                        // Add tag if not present
                        let tag = #tag;
                        if !tag.is_empty() {
                           operation.tags.get_or_insert_with(Vec::new).push(tag.to_string());
                        }

                        // Construct PathItem
                        // In Utoipa 5, PathItem::new takes (method, operation)
                        let mut methods_iter = methods.into_iter();
                        let first_method = methods_iter.next().expect("At least one method required");

                        let mut item = utoipa::openapi::path::PathItem::new(
                            first_method.clone(),
                            operation.clone()
                        );

                        // Add remaining methods if any
                        for method in methods_iter {
                             match method {
                                utoipa::openapi::path::HttpMethod::Get => item.get = Some(operation.clone()),
                                utoipa::openapi::path::HttpMethod::Post => item.post = Some(operation.clone()),
                                utoipa::openapi::path::HttpMethod::Put => item.put = Some(operation.clone()),
                                utoipa::openapi::path::HttpMethod::Delete => item.delete = Some(operation.clone()),
                                utoipa::openapi::path::HttpMethod::Options => item.options = Some(operation.clone()),
                                utoipa::openapi::path::HttpMethod::Head => item.head = Some(operation.clone()),
                                utoipa::openapi::path::HttpMethod::Patch => item.patch = Some(operation.clone()),
                                utoipa::openapi::path::HttpMethod::Trace => item.trace = Some(operation.clone()),
                            }
                        }

                        // Merge or insert
                        if let Some(existing) = openapi.paths.paths.get_mut(&path) {
                             if let Some(op) = item.get { existing.get = Some(op); }
                             if let Some(op) = item.post { existing.post = Some(op); }
                             if let Some(op) = item.put { existing.put = Some(op); }
                             if let Some(op) = item.delete { existing.delete = Some(op); }
                             if let Some(op) = item.options { existing.options = Some(op); }
                             if let Some(op) = item.head { existing.head = Some(op); }
                             if let Some(op) = item.patch { existing.patch = Some(op); }
                             if let Some(op) = item.trace { existing.trace = Some(op); }
                        } else {
                             openapi.paths.paths.insert(path, item);
                        }
                    }
                )*
            }
        }
    };

    // Generate utoipa wrapper module
    let utoipa_module = {
        // Create list of function names as strings for documentation
        let fn_names: Vec<_> = routes
            .iter()
            .map(|(fn_name, _, _)| fn_name.to_string())
            .collect();
        let fn_count = fn_names.len();

        quote! {
            /// Module containing utoipa-compatible wrapper functions for OpenAPI documentation.
            ///
            /// These functions are stubs that provide metadata for the OpenAPI spec generator.
            /// The actual implementations are in the controller impl block above.
            ///
            /// ## Usage in OpenApi derive:
            /// ```ignore
            /// use crate::controller::my_controller::__UTOIPA_PATHS__;
            ///
            /// #[derive(OpenApi)]
            /// #[openapi(paths(
            ///     __UTOIPA_PATHS__::handler1,
            ///     __UTOIPA_PATHS__::handler2,
            /// ))]
            /// struct ApiDoc;
            /// ```
            #[doc(hidden)]
            pub mod __UTOIPA_PATHS__ {
                use super::*;
                use eywa_axum::prelude::*;

                /// List of path function names in this controller
                pub const PATH_NAMES: [&str; #fn_count] = [#(#fn_names),*];

                /// Number of paths in this controller
                pub const PATH_COUNT: usize = #fn_count;

                #(#utoipa_wrappers)*
            }
        }
    };

    quote! {
        #impl_block

        #into_router_impl

        #utoipa_module
    }
}

/// Parse a #[route(...)] attribute into RouteInfo
fn parse_route_attr(attr: &Attribute) -> syn::Result<RouteInfo> {
    let tokens = attr.meta.require_list()?.tokens.clone();
    RouteInfo::parse(tokens)
}

/// Helper to extract T from Result<Json<T>> or Json<T> return types
fn extract_inner_type(ty: &syn::Type) -> Option<syn::Type> {
    if let syn::Type::Path(tp) = ty {
        // Check if it matches Result<...>
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                        // We found the Success type of Result. Check if it's Json<T>
                        return extract_json_type(inner);
                    }
                }
            }
            // Check if it matches Json<...>
            if seg.ident == "Json" {
                return extract_json_type(ty);
            }
        }
    }
    None
}

fn extract_json_type(ty: &syn::Type) -> Option<syn::Type> {
    if let syn::Type::Path(tp) = ty {
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident == "Json" {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                        return Some(inner.clone());
                    }
                }
            }
        }
    }
    // If not Json<...>, return None as we only support wrapping Json responses for now
    None
}

fn extract_hateoas_inner_type(ty: &syn::Type) -> Option<&syn::Type> {
    if let syn::Type::Path(tp) = ty {
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident == "HateoasResponse" {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                        return Some(inner);
                    }
                }
            }
        }
    }
    None
}

/// Helper to extract variable identifiers from a pattern (e.g., extract 'id' from 'Path(id)')
fn collect_pat_idents(pat: &syn::Pat) -> Vec<syn::Ident> {
    match pat {
        syn::Pat::Ident(p) => vec![p.ident.clone()],
        syn::Pat::TupleStruct(p) => p.elems.iter().flat_map(collect_pat_idents).collect(),
        syn::Pat::Type(p) => collect_pat_idents(&p.pat),
        syn::Pat::Tuple(p) => p.elems.iter().flat_map(collect_pat_idents).collect(),
        syn::Pat::Reference(p) => collect_pat_idents(&p.pat),
        _ => Vec::new(),
    }
}
