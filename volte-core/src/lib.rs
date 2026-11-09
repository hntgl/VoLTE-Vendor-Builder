//! Core library of the VoLTE vendor image builder.
//!
//! Every module of this crate is pure logic: it never writes to stdout or
//! stderr. Callers (the CLI, later an app shell) pass logging sinks in.

#![forbid(unsafe_code)]

pub mod avb;
pub mod chip_detect;
pub mod config;
pub mod env;
pub mod error;
pub mod ext4_tools;
pub mod flags;
pub mod init_rc;
pub mod logging;
pub mod modem;
pub mod path_id;
pub mod payload;
pub mod pipeline;
pub mod profile;
pub mod props;
pub mod report;
pub mod rustfs_ext4;
pub mod sha1;
pub mod sha256;
pub mod slot;
pub mod sparse;
pub mod verify;
