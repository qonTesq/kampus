//! Safe output: HTML stripping, caps, and schema-checked envelope projection.

mod project;
mod sanitize;

pub use project::{is_error_payload, project};
pub use sanitize::{cap_items, clean_text};

/// Frozen output limits.
pub mod limits {
    /// Titles and names, in Unicode scalar values.
    pub const TITLE_MAX: usize = 160;
    /// Public descriptions and notes.
    pub const DESCRIPTION_MAX: usize = 500;
    /// Free-text query arguments.
    pub const QUERY_MAX: usize = 120;
    /// Default list length.
    pub const LIST_DEFAULT: usize = 20;
    /// Hard maximum list length.
    pub const LIST_MAX: usize = 50;
    /// Serialized `structuredContent` size.
    pub const RESPONSE_MAX_BYTES: usize = 32 * 1024;
}
