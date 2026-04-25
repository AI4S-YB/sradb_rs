//! sradb-core — core types, HTTP client, and parsers for the sradb-rs project.
//!
//! See `docs/superpowers/specs/2026-04-25-sradb-rs-design.md` for the full spec.

pub mod accession;
pub mod client;
pub mod error;
pub mod http;

pub use accession::{Accession, AccessionKind, ParseAccessionError};
pub use client::{ClientConfig, SraClient};
pub use error::{Result, SradbError};
