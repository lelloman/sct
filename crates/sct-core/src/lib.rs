//! Application operations live here, independently of HTTP handlers.
//!
//! The catalog owns metadata persistence. Future storage adapters implement byte
//! operations; a separate placement coordinator composes destinations. Provider
//! settings and registration must not leak into namespace or retention rules.

pub mod catalog;

pub mod auth;
pub mod backend;
pub mod error;
mod grants;
pub mod lease;
mod namespace;
pub mod operations;
mod paging;
pub mod policy;
pub mod roots;

pub mod accounting;

pub mod content_catalog;

pub mod payload;

pub mod local_store;

pub mod s3_store;

pub mod transfer_io;

mod upload;

mod admission;
