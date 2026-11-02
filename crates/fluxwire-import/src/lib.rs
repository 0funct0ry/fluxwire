//! Importers for Postman, Insomnia, OpenAPI, Hurl and `.http` files (SPEC §14).
//!
//! Every importer builds an in-memory [`ImportPlan`] (files to create plus an
//! [`ImportReport`]) and writes nothing. [`apply`] writes a plan into a target folder, never
//! overwriting an existing file unless the caller confirms that file.

mod build;
mod httpfile;
mod hurl;
mod insomnia;
mod openapi;
mod plan;
mod postman;
mod render;
mod scripts;
mod secrets;

pub use plan::{
    ImportError, ImportPlan, ImportReport, Lossy, Options, PlannedFile, PlannedKind, Source, apply,
    detect, plan,
};
