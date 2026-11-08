//! Shared HTTP transport (ADR-025): hyper + rustls engine, auth, OAuth2 and the resolved
//! `HttpPlan`. Used by `fluxwire-http` and `fluxwire-graphql`, which never depend on each other.

mod auth;
mod body;
pub mod curl;
mod engine;
mod error;
pub mod expect;
mod fetch;
pub mod history;
pub mod oauth2;
mod plan;
pub mod script;
mod tls;

pub use auth::{Auth, OAuth2Config, digest_header, parse_challenge};
pub use engine::run;
pub use error::*;
pub use fetch::get_text;
pub use plan::{HttpPlan, SettingsOverride, plan};
