use serde::{Deserialize, Serialize};
use std::fmt;

/// A React/Next.js module-level directive (`"use client"` / `"use server"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Directive {
    None,
    UseClient,
    UseServer,
}

impl Directive {
    pub fn as_str(self) -> &'static str {
        match self {
            Directive::None => "none",
            Directive::UseClient => "use client",
            Directive::UseServer => "use server",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "use client" => Directive::UseClient,
            "use server" => Directive::UseServer,
            _ => Directive::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    /// Capitalised function (heuristic: we do not yet verify it returns JSX).
    Component,
    /// `useXxx` function.
    Hook,
    Class,
    Variable,
    Type,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Component => "component",
            SymbolKind::Hook => "hook",
            SymbolKind::Class => "class",
            SymbolKind::Variable => "variable",
            SymbolKind::Type => "type",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "component" => SymbolKind::Component,
            "hook" => SymbolKind::Hook,
            "class" => SymbolKind::Class,
            "variable" => SymbolKind::Variable,
            "type" => SymbolKind::Type,
            _ => SymbolKind::Function,
        }
    }
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub exported: bool,
    pub is_default: bool,
    /// 1-based, inclusive.
    pub start_line: u32,
    pub end_line: u32,
}

/// One imported binding. `imported` is the exported name, `"default"` or `"*"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Specifier {
    pub imported: String,
    pub local: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub source: String,
    /// Project-relative path if the import points at a file inside the project.
    pub resolved: Option<String>,
    pub specifiers: Vec<Specifier>,
    pub line: u32,
}

/// Everything extracted from one source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIndex {
    /// Project-relative, forward slashes.
    pub path: String,
    pub hash: String,
    pub directive: Directive,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
}
