//! goulash as a library.
//!
//! The binary (`main.rs`) is a thin wrapper over [`session::run`]. This
//! surface exists so out-of-process tooling — specifically the
//! characterization bench — can drive the **real** engine: the real
//! prompt assembly, the real wire formats, the real answer parser.
//!
//! That is the whole point of it. A harness that reimplements prompt
//! building measures the reimplementation, and the two drift the moment
//! either is touched. Every number in `bench/` is only worth reading
//! because it came through this door.

pub mod config;
pub mod configcli;
pub mod context;
pub mod engine;
pub mod facts;
pub mod integrate;
pub mod memory;
pub mod models;
pub mod osc;
pub mod pincache;
pub mod pty;
pub mod record;
pub mod sense;
pub mod session;
pub mod state;
pub mod stats;
pub mod status;
pub mod term;
pub mod vendor;
pub mod wire;
