//! Application operations live here, independently of HTTP handlers.
//!
//! The catalog owns metadata persistence. Future storage adapters implement byte
//! operations; a separate placement coordinator composes destinations. Provider
//! settings and registration must not leak into namespace or retention rules.

pub mod catalog;
pub mod checkpoint;
pub mod checkpoint_export;

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

mod ingest;

mod publication;

mod deletion;
pub mod download;
mod graph_work;
mod lifecycle;
mod namespace_moves;
mod policy_preview;
mod tree_build;
mod tree_catalog;
mod tree_draft;

mod purge;

mod management;

mod storage_management;

mod inspection;

pub mod maintenance;

mod integrity;

mod mirror_repair;
