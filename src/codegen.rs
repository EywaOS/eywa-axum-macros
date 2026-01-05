//! Code generation utilities

use proc_macro2::TokenStream;
use quote::quote;

/// Generates the IntoRouter trait implementation
#[allow(dead_code)]
pub fn generate_into_router_trait() -> TokenStream {
    quote! {
        /// Trait for converting a controller into an axum Router.
        pub trait IntoRouter<S>
        where
            S: Clone + Send + Sync + 'static,
        {
            /// Creates an axum Router from this controller.
            fn into_router(state: S) -> ::axum::Router<S>;

            /// Returns the URL prefix for this controller.
            fn prefix() -> &'static str;

            /// Returns the OpenAPI tag for this controller.
            fn tag() -> &'static str;
        }
    }
}
