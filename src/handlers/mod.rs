//! HTTP request handlers for pages, JSON API, and chat endpoints.

mod api;
mod chat;
mod pages;

pub use api::*;
pub use chat::*;
pub use pages::*;
