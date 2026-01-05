//! # eywa-axum-controller-macros
//!
//! Procedural macros for the eywa-axum-controller framework.
//!
//! This crate provides:
//! - `#[controller]` - Define a controller with routes, middleware, and OpenAPI metadata
//! - `#[route]` - Define individual routes with HTTP method, path, and documentation
//! - `openapi_for!` - Generate OpenAPI documentation struct (experimental)
//!
//! ## Features
//!
//! ### Route Attributes
//! - `tag = "..."` - Single OpenAPI tag (legacy)
//! - `tags = ["...", "..."]` - Multiple OpenAPI tags
//! - `summary = "..."` - Route summary
//! - `description = "..."` - Route description
//! - `deprecated` - Mark as deprecated
//! - `security` - Require bearer authentication

mod codegen;
mod controller;
mod openapi;
mod parse;
mod route;

use proc_macro::TokenStream;

/// Marks an impl block as a controller.
///
/// # Attributes
/// - `path` - URL prefix for all routes (preferred)
/// - `prefix` - URL prefix for all routes (legacy, use `path` instead)
/// - `state` - The application state type (required)
/// - `tag` - OpenAPI tag for grouping (default: controller name)
/// - `middleware` - Middleware function to apply
///
/// # Example
/// ```ignore
/// #[controller(
///     path = "/projects",
///     state = AppState,
///     tag = "Projects"
/// )]
/// impl ProjectsController {
///     #[route(GET "/")]
///     async fn list(State(state): State<AppState>) -> Json<Vec<Project>> {
///         // ...
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn controller(args: TokenStream, input: TokenStream) -> TokenStream {
    controller::controller_impl(args.into(), input.into()).into()
}

/// Marks a function as a route handler.
///
/// # Syntax
/// `#[route(METHOD "/path")]`
///
/// # Attributes
/// - `summary` - OpenAPI summary
/// - `description` - OpenAPI description
/// - `tag` - Single OpenAPI tag (legacy)
/// - `tags` - Multiple OpenAPI tags: `tags = ["Tag1", "Tag2"]`
/// - `security` - Require bearer authentication
/// - `deprecated` - Mark as deprecated
/// - `collection` - Wrap response in CollectionResponse (future)
/// - `hateoas` - Wrap response in HateoasResponse (future)
///
/// # Example
/// ```ignore
/// #[route(GET "/:id", summary = "Get project by ID", tags = ["Projects", "Admin"])]
/// async fn get(
///     State(state): State<AppState>,
///     Path(id): Path<Uuid>,
/// ) -> Result<Json<Project>> {
///     // ...
/// }
/// ```
#[proc_macro_attribute]
pub fn route(args: TokenStream, input: TokenStream) -> TokenStream {
    route::route_impl(args.into(), input.into()).into()
}

/// Generate OpenAPI documentation struct (experimental).
///
/// This macro helps generate the OpenAPI documentation struct by combining
/// multiple controllers and their schemas.
///
/// # Example
/// ```ignore
/// eywa_axum::openapi_for! {
///     controllers = [TimerController, ProjectController],
///     schemas = [ToggleTimerRequest, TimerStatusResponse],
///     tags = [
///         (name = "Timer", description = "Timer management"),
///     ],
///     info = (
///         title = "My API",
///         version = "1.0.0",
///     )
/// }
/// ```
///
/// # Note
/// Due to proc macro limitations, individual paths still need to be listed
/// manually in the `#[openapi(paths(...))]` attribute. This macro primarily
/// helps with organization and provides a consistent pattern.
#[proc_macro]
pub fn openapi_for(input: TokenStream) -> TokenStream {
    openapi::openapi_for_impl(input.into()).into()
}
