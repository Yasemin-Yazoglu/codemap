//! Syntax-level extraction (tree-sitter) and import resolution for TS/JS/TSX/JSX.
mod extract;
mod hash;
mod resolve;

pub use extract::extract;
pub use hash::content_hash;
pub use resolve::Resolver;

/// File extensions the indexer treats as source code.
pub const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
