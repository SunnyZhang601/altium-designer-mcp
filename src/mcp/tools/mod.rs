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

/// The error for a component a tool was asked for and the library does not
/// hold: the request as made, then what is there — the first ten names and
/// a count of the rest — in the same words from every tool that looks one
/// up.
pub fn component_not_found(component_name: &str, names: &[String]) -> String {
    component_not_found_in(component_name, "library", names)
}

/// [`component_not_found`] for a tool that holds more than one library,
/// where `which` says which one was searched ("source library", a file
/// name the caller passed).
pub fn component_not_found_in(component_name: &str, which: &str, names: &[String]) -> String {
    const SHOWN: usize = 10;
    let available = if names.is_empty() {
        "none (the library is empty)".to_string()
    } else {
        let shown = names
            .iter()
            .take(SHOWN)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        match names.len().saturating_sub(SHOWN) {
            0 => shown,
            rest => format!("{shown} ... and {rest} more"),
        }
    };
    format!("Component '{component_name}' not found in {which}. Available: {available}")
}

#[cfg(test)]
mod tests {
    use super::{component_not_found, component_not_found_in};

    /// The message names the request, then the first ten names on file and
    /// how many more there are; an empty library says so.
    #[test]
    fn a_missing_component_is_reported_with_what_is_there() {
        let names = |n: usize| (1..=n).map(|i| format!("C{i}")).collect::<Vec<_>>();
        assert_eq!(
            component_not_found("X", &names(0)),
            "Component 'X' not found in library. Available: none (the library is empty)"
        );
        assert_eq!(
            component_not_found("X", &names(2)),
            "Component 'X' not found in library. Available: C1, C2"
        );
        assert_eq!(
            component_not_found("X", &names(10)),
            "Component 'X' not found in library. Available: C1, C2, C3, C4, C5, C6, C7, C8, C9, C10"
        );
        assert_eq!(
            component_not_found("x", &names(12)),
            "Component 'x' not found in library. Available: C1, C2, C3, C4, C5, C6, C7, C8, C9, C10 \
             ... and 2 more"
        );
        assert_eq!(
            component_not_found_in("X", "source library", &names(1)),
            "Component 'X' not found in source library. Available: C1"
        );
    }
}
