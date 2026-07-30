//! goulash as a library.
//!
//! The binary (`main.rs`) is a thin wrapper over [`session::run`]. The
//! library surface exists so out-of-process tooling — notably the
//! characterization bench — can drive the *real* engine (prompt
//! assembly, provider I/O, answer parsing) rather than a reimplementation
//! of it that would drift.

pub mod config;
pub mod configcli;
pub mod engine;
pub mod facts;
pub mod integrate;
pub mod memory;
pub mod osc;
pub mod pty;
pub mod record;
pub mod sense;
pub mod session;
pub mod state;
pub mod status;
pub mod term;
pub mod vendor;
