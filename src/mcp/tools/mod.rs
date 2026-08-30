//! Per-domain MCP tool handlers and helpers, split out of `server.rs`.
//!
//! Each submodule adds an `impl McpServer` block. Method resolution is
//! independent of which file an `impl` lives in, so the dispatch in `server.rs`
//! (and the in-crate tests) call these methods unchanged via `Self::`/`self.`.
//! Helpers reached across modules are `pub(crate)`.

mod allowed_keys;
mod batch;
mod compare;
mod components;
mod diff;
mod library_ops;
mod maintenance;
#[cfg(test)]
mod mutation_fidelity;
mod parsing;
/// The primitive kinds `update_primitive` addresses, shared by its handler,
/// its schema and the guard test.
pub(super) use maintenance::UPDATE_PRIMITIVE_KINDS;
/// The accepted values of every enum-valued field, shared by the parsers
/// that read them and the tool schemas that advertise them.
pub(super) use parsing::accepted;
mod query_update;
mod read_write;
mod render;
mod schlib_manage;
mod step;
#[cfg(test)]
pub mod test_support;
mod validation;
