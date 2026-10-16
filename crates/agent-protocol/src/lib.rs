//! Pure Agent Client Protocol (ACP) client logic.
//!
//! No processes, threads, filesystem or UI live here. The service layer feeds
//! bytes in and writes bytes out; the app layer decides what messages mean for
//! the user. Everything is tolerant of fields and update kinds this version does
//! not know: ACP agents (and the adapters in front of them) add extensions often.
pub mod framing;
pub mod handshake;
pub mod reducer;
pub mod requests;
pub mod rpc;
pub mod types;
