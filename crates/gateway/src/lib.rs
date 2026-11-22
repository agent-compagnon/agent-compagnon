//! Tool registry, upstream clients, middleware chain, MCP server, bridge, and the assembly the application runs.

pub mod bridge;
pub mod control;
mod convert;
pub mod interface;
pub mod journal;
pub mod journal_view;
pub mod launch;
pub mod modules;
pub mod notify;
pub mod registry;
pub mod runtime;
pub mod self_update;
pub mod server;
pub mod updates;
pub mod upstream;
pub mod view;
