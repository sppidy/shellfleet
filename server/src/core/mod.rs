pub mod events;
pub mod http;
pub mod model;
pub mod repository;

pub use events::CoreEventBus;

pub fn routes() -> axum::Router<std::sync::Arc<crate::AppState>> {
    http::routes()
}
