use crate::hash::content_hash;
use anyhow::{anyhow, Result};
use codemap_core::model::*;
use tree_sitter::{Language, Node, Parser};

fn language_for(path: &str) -> Language {
    // `.ts` must not use the TSX grammar: `<T>expr` casts are ambiguous with JSX.
    if path.ends_with(".ts") || path.ends_with(".mts") || path.ends_with(".cts") {
        tree_sitter_typescript::language_typescript()
    } else {
        tree_sitter_typescript::language_tsx()
    }
}

/// Parse one file and extract its directive, top-level symbols and imports.
/// `path` is project-relative and only used to pick the grammar and label the result.
/// Import `resolved` fields are left empty; see [`crate::Resolver`].
pub fn extract(path: &str, source: &str) -> Result<FileIndex> {
    let mut parser = Parser::new();
    parser
        .set_language(&language_for(path))
        .map_err(|e| anyhow!("loading grammar: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("tree-sitter failed to parse {path}"))?;
    let root = tree.root_node();

    let mut ex = Extractor {
        src: source.as_bytes(),
        symbols: Vec::new(),
        imports: Vec::new(),
        pending: Vec::new(),
    };
    for i in 0..root.named_child_count() {
        let Some(node) = root.named_child(i) else {
            continue;
        };
        match node.kind() {
            "import_statement" => ex.import(node),
            "export_statement" => ex.export(node),
            _ => ex.declaration(node, false, false),
        }
    }
    ex.apply_pending();

    Ok(FileIndex {
        path: path.to_string(),
        hash: content_hash(source),
        directive: directive_of(root, source.as_bytes()),
        symbols: ex.symbols,
        imports: ex.imports,
    })
}

fn directive_of(root: Node, src: &[u8]) -> Directive {
    for i in 0..root.named_child_count() {
        let Some(n) = root.named_child(i) else { break };
        match n.kind() {
            "comment" => continue,
            "expression_statement" => {
                let Some(s) = n.named_child(0).filter(|s| s.kind() == "string") else {
                    break;
                };
                let text = s
                    .utf8_text(src)
                    .unwrap_or("")
                    .trim_matches(|c| c == '"' || c == '\'');
                match text {
                    "use client" => return Directive::UseClient,
                    "use server" => return Directive::UseServer,
                    _ => continue, // e.g. "use strict": the directive prologue may continue
                }
            }
            _ => break,
        }
    }
    Directive::None
}

fn function_kind(name: &str) -> SymbolKind {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_uppercase() => SymbolKind::Component,
        Some('u')
            if name.starts_with("use") && name[3..].starts_with(|c: char| c.is_uppercase()) =>
        {
            SymbolKind::Hook
        }
        _ => SymbolKind::Function,
    }
}

struct Extractor<'a> {
    src: &'a [u8],
    symbols: Vec<Symbol>,
    imports: Vec<Import>,
    /// `(local name, exported-as name)` from `export { a as b }` / `export default a`.
    pending: Vec<(String, String)>,
}

impl<'a> Extractor<'a> {
    fn text(&self, n: Node) -> &'a str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn field_text(&self, n: Node, field: &str) -> Option<String> {
        n.child_by_field_name(field)
            .map(|c| self.text(c).to_string())
    }

    fn push(
        &mut self,
        name: String,
        kind: SymbolKind,
        span: Node,
        exported: bool,
        is_default: bool,
    ) {
        self.symbols.push(Symbol {
            name,
            kind,
            exported,
            is_default,
            start_line: span.start_position().row as u32 + 1,
            end_line: span.end_position().row as u32 + 1,
        });
    }

    fn declaration(&mut self, node: Node, exported: bool, is_default: bool) {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                let name = self
                    .field_text(node, "name")
                    .unwrap_or_else(|| "default".into());
                let kind = function_kind(&name);
                self.push(name, kind, node, exported, is_default);
            }
            "class_declaration" | "abstract_class_declaration" => {
                let name = self
                    .field_text(node, "name")
                    .unwrap_or_else(|| "default".into());
                self.push(name, SymbolKind::Class, node, exported, is_default);
            }
            "interface_declaration" | "type_alias_declaration" => {
                if let Some(name) = self.field_text(node, "name") {
                    self.push(name, SymbolKind::Type, node, exported, is_default);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                let mut cursor = node.walk();
                for d in node.named_children(&mut cursor) {
                    if d.kind() != "variable_declarator" {
                        continue;
                    }
                    let Some(name_node) = d.child_by_field_name("name") else {
                        continue;
                    };
                    if name_node.kind() != "identifier" {
                        continue; // destructuring: not tracked yet
                    }
                    let name = self.text(name_node).to_string();
                    let kind = d
                        .child_by_field_name("value")
                        .map_or(SymbolKind::Variable, |v| self.value_kind(v, &name));
                    self.push(name, kind, d, exported, is_default);
                }
            }
            _ => {}
        }
    }

    fn value_kind(&self, value: Node, name: &str) -> SymbolKind {
        match value.kind() {
            "arrow_function" | "function_expression" | "function" => function_kind(name),
            "call_expression" => {
                let callee = value
                    .child_by_field_name("function")
                    .map_or("", |c| self.text(c));
                if matches!(
                    callee,
                    "memo" | "forwardRef" | "React.memo" | "React.forwardRef"
                ) {
                    SymbolKind::Component
                } else {
                    SymbolKind::Variable
                }
            }
            _ => SymbolKind::Variable,
        }
    }

    fn import(&mut self, node: Node) {
        let Some(source_node) = node.child_by_field_name("source") else {
            return;
        };
        let source = self
            .text(source_node)
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        let mut specifiers = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "import_clause" {
                continue;
            }
            let mut c2 = child.walk();
            for part in child.children(&mut c2) {
                match part.kind() {
                    "identifier" => {
                        let local = self.text(part).to_string();
                        specifiers.push(Specifier {
                            imported: "default".into(),
                            local,
                        });
                    }
                    "named_imports" => {
                        let mut c3 = part.walk();
                        for spec in part.named_children(&mut c3) {
                            if spec.kind() != "import_specifier" {
                                continue;
                            }
                            let Some(imported) = self.field_text(spec, "name") else {
                                continue;
                            };
                            let local = self
                                .field_text(spec, "alias")
                                .unwrap_or_else(|| imported.clone());
                            specifiers.push(Specifier { imported, local });
                        }
                    }
                    "namespace_import" => {
                        let mut c3 = part.walk();
                        let local = part
                            .named_children(&mut c3)
                            .find(|n| n.kind() == "identifier")
                            .map(|n| self.text(n).to_string())
                            .unwrap_or_default();
                        specifiers.push(Specifier {
                            imported: "*".into(),
                            local,
                        });
                    }
                    _ => {}
                }
            }
        }
        self.imports.push(Import {
            source,
            resolved: None,
            specifiers,
            line: node.start_position().row as u32 + 1,
        });
    }

    fn export(&mut self, node: Node) {
        let is_default = {
            let mut c = node.walk();
            let found = node.children(&mut c).any(|ch| ch.kind() == "default");
            found
        };

        // Re-export: `export { a } from './x'` / `export * from './x'`.
        if let Some(source_node) = node.child_by_field_name("source") {
            let source = self
                .text(source_node)
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            let mut specifiers = Vec::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "*" | "namespace_export" => specifiers.push(Specifier {
                        imported: "*".into(),
                        local: "*".into(),
                    }),
                    "export_clause" => {
                        let mut c2 = child.walk();
                        for spec in child.named_children(&mut c2) {
                            let Some(name) = self.field_text(spec, "name") else {
                                continue;
                            };
                            let local = self
                                .field_text(spec, "alias")
                                .unwrap_or_else(|| name.clone());
                            specifiers.push(Specifier {
                                imported: name,
                                local,
                            });
                        }
                    }
                    _ => {}
                }
            }
            self.imports.push(Import {
                source,
                resolved: None,
                specifiers,
                line: node.start_position().row as u32 + 1,
            });
            return;
        }

        if let Some(decl) = node.child_by_field_name("declaration") {
            self.declaration(decl, true, is_default);
            return;
        }

        if let Some(value) = node.child_by_field_name("value") {
            match value.kind() {
                "identifier" => self
                    .pending
                    .push((self.text(value).to_string(), "default".into())),
                "arrow_function" | "function_expression" | "function" => {
                    self.push("default".into(), SymbolKind::Function, node, true, true)
                }
                _ => {}
            }
            return;
        }

        // `export { a, b as c }`
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != "export_clause" {
                continue;
            }
            let mut c2 = child.walk();
            for spec in child.named_children(&mut c2) {
                let Some(name) = self.field_text(spec, "name") else {
                    continue;
                };
                let alias = self
                    .field_text(spec, "alias")
                    .unwrap_or_else(|| name.clone());
                self.pending.push((name, alias));
            }
        }
    }

    /// Resolve `export { x }` / `export default x` against symbols declared anywhere in the file.
    fn apply_pending(&mut self) {
        for (local, exported_as) in std::mem::take(&mut self.pending) {
            if let Some(sym) = self.symbols.iter_mut().find(|s| s.name == local) {
                sym.exported = true;
                if exported_as == "default" {
                    sym.is_default = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym<'a>(f: &'a FileIndex, name: &str) -> &'a Symbol {
        f.symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no symbol {name}"))
    }

    #[test]
    fn directive_detection() {
        let f = extract(
            "a.tsx",
            "'use client';\nexport function A() { return <div/>; }",
        )
        .unwrap();
        assert_eq!(f.directive, Directive::UseClient);
        let f = extract(
            "a.ts",
            "// c\n\"use strict\";\n\"use server\";\nexport async function act() {}",
        )
        .unwrap();
        assert_eq!(f.directive, Directive::UseServer);
        let f = extract("a.ts", "const x = 1;\n\"use client\";").unwrap();
        assert_eq!(
            f.directive,
            Directive::None,
            "directive must be in the prologue"
        );
    }

    #[test]
    fn symbol_kinds_and_spans() {
        let src = "export function Page() { return <div/>; }\n\
                   export const useCart = () => 1;\n\
                   const helper = (x: number) => x;\n\
                   export const Chart = memo(function Chart() { return null; });\n\
                   export const COLORS = ['a'];\n\
                   export interface Props { a: string }\n\
                   class Foo {}\n";
        let f = extract("a.tsx", src).unwrap();
        assert_eq!(sym(&f, "Page").kind, SymbolKind::Component);
        assert_eq!(
            (sym(&f, "Page").start_line, sym(&f, "Page").end_line),
            (1, 1)
        );
        assert_eq!(sym(&f, "useCart").kind, SymbolKind::Hook);
        assert_eq!(sym(&f, "helper").kind, SymbolKind::Function);
        assert!(!sym(&f, "helper").exported);
        assert_eq!(sym(&f, "Chart").kind, SymbolKind::Component);
        assert_eq!(sym(&f, "COLORS").kind, SymbolKind::Variable);
        assert_eq!(sym(&f, "Props").kind, SymbolKind::Type);
        assert_eq!(sym(&f, "Foo").kind, SymbolKind::Class);
    }

    #[test]
    fn default_and_named_exports() {
        let f = extract("a.tsx", "export default function Home() { return null }").unwrap();
        assert!(sym(&f, "Home").exported && sym(&f, "Home").is_default);

        let f = extract(
            "a.ts",
            "function a() {}\nfunction b() {}\nexport { a, b as c };\nexport default a;",
        )
        .unwrap();
        assert!(sym(&f, "a").exported && sym(&f, "a").is_default);
        assert!(sym(&f, "b").exported && !sym(&f, "b").is_default);
    }

    #[test]
    fn imports_all_shapes() {
        let src = "import React, { useState as useS, type Foo } from 'react';\n\
                   import * as db from \"@/lib/db\";\n\
                   import './global.css';\n\
                   export { x as y } from './x';\n\
                   export * from './all';\n";
        let f = extract("a.tsx", src).unwrap();
        assert_eq!(f.imports.len(), 5);
        let react = &f.imports[0];
        assert_eq!(react.source, "react");
        assert_eq!(
            react.specifiers[0],
            Specifier {
                imported: "default".into(),
                local: "React".into()
            }
        );
        assert_eq!(
            react.specifiers[1],
            Specifier {
                imported: "useState".into(),
                local: "useS".into()
            }
        );
        assert_eq!(react.specifiers[2].imported, "Foo");
        assert_eq!(
            f.imports[1].specifiers[0],
            Specifier {
                imported: "*".into(),
                local: "db".into()
            }
        );
        assert!(f.imports[2].specifiers.is_empty());
        assert_eq!(
            f.imports[3].specifiers[0],
            Specifier {
                imported: "x".into(),
                local: "y".into()
            }
        );
        assert_eq!(f.imports[4].specifiers[0].imported, "*");
        assert_eq!(f.imports[4].line, 5);
    }

    #[test]
    fn ts_files_allow_angle_bracket_generics() {
        let f = extract(
            "a.ts",
            "export const id = <T,>(x: T): T => x;\nexport function f<T>(x: T) { return <T>x; }",
        )
        .unwrap();
        assert_eq!(f.symbols.len(), 2);
    }

    #[test]
    fn broken_code_still_yields_symbols() {
        let f = extract(
            "a.tsx",
            "export function Ok() { return 1 }\nexport function Broken( {",
        )
        .unwrap();
        assert!(f.symbols.iter().any(|s| s.name == "Ok"));
    }
}
