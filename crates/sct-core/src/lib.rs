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
pub mod operations;
mod paging;
pub mod roots;
