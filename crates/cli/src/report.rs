//! Human-readable output for the CLI commands.
use anyhow::Result;
use codemap_core::model::{Directive, Symbol};
use codemap_core::store::Store;
use codemap_nextjs::{analyze, InterceptorKind, RouteFile, SourceFile};
use std::fmt::Write;

fn tag(d: Directive) -> &'static str {
    match d {
        Directive::UseClient => " [client]",
        Directive::UseServer => " [server actions]",
        Directive::None => "",
    }
}

fn describe(f: &RouteFile) -> String {
    format!("{}{}", f.path, tag(f.directive))
}

pub fn routes(store: &Store) -> Result<String> {
    let files: Vec<SourceFile> = store
        .file_summaries()?
        .into_iter()
        .map(|s| SourceFile {
            path: s.path,
            directive: s.directive,
            exports: s.exports,
        })
        .collect();
    let app = analyze(&files);
    let mut out = String::new();

    match &app.app_root {
        Some(root) => writeln!(out, "App Router: {root}/")?,
        None => {
            writeln!(out, "No App Router directory (`app/` or `src/app/`) found.")?;
        }
    }
    if let Some(i) = &app.interceptor {
        let kind = match i.kind {
            InterceptorKind::Proxy => "proxy",
            InterceptorKind::Middleware => "middleware (legacy name)",
        };
        writeln!(
            out,
            "Request interceptor: {} ({kind}); matcher analysis not implemented yet",
            i.path
        )?;
    }
    if app.routes.is_empty() {
        return Ok(out);
    }
    writeln!(out)?;
    let width = app
        .routes
        .iter()
        .map(|r| r.url.len())
        .max()
        .unwrap_or(0)
        .max(8);
    for r in &app.routes {
        let methods = if r.methods.is_empty() {
            String::new()
        } else {
            format!("  {{{}}}", r.methods.join(", "))
        };
        writeln!(out, "{:<width$}  {}{}", r.url, describe(&r.entry), methods)?;
        if !r.layouts.is_empty() {
            let l: Vec<_> = r.layouts.iter().map(describe).collect();
            writeln!(out, "{:<width$}    layouts: {}", "", l.join(" > "))?;
        }
        if !r.boundaries.is_empty() {
            let b: Vec<_> = r
                .boundaries
                .iter()
                .map(|b| format!("{} {}", b.kind.as_str(), b.path))
                .collect();
            writeln!(out, "{:<width$}    also:    {}", "", b.join(", "))?;
        }
    }
    Ok(out)
}

pub fn show(store: &Store, name: &str) -> Result<String> {
    let mut found = store.find_symbols(name, false)?;
    let mut fuzzy = false;
    if found.is_empty() {
        found = store.find_symbols(name, true)?;
        fuzzy = true;
    }
    let mut out = String::new();
    if found.is_empty() {
        writeln!(
            out,
            "No symbol matching `{name}`. Run `codemap index` if the project changed."
        )?;
        return Ok(out);
    }
    if fuzzy {
        writeln!(out, "No exact match for `{name}`; closest by substring:\n")?;
    }
    for (file, sym) in &found {
        write_symbol(&mut out, store, file, sym)?;
    }
    Ok(out)
}

fn write_symbol(out: &mut String, store: &Store, file: &str, sym: &Symbol) -> Result<()> {
    let dir = store.directive_of(file)?;
    let mut flags = Vec::new();
    if sym.is_default {
        flags.push("default export");
    } else if sym.exported {
        flags.push("exported");
    } else {
        flags.push("local");
    }
    writeln!(out, "{} {}  ({})", sym.kind, sym.name, flags.join(", "))?;
    writeln!(
        out,
        "  {file}:{}-{}{}",
        sym.start_line,
        sym.end_line,
        tag(dir)
    )?;

    if !sym.exported {
        writeln!(
            out,
            "  used only inside this file (cross-file usage needs the semantic pass)\n"
        )?;
        return Ok(());
    }
    let importers: Vec<_> = store
        .importers_of(file)?
        .into_iter()
        .filter_map(|imp| {
            let hits: Vec<String> = imp
                .specifiers
                .iter()
                .filter(|s| {
                    s.imported == "*"
                        || s.imported == sym.name
                        || (sym.is_default && s.imported == "default")
                })
                .map(|s| {
                    if s.imported == "*" {
                        format!("namespace `{}`", s.local)
                    } else if s.local != sym.name {
                        format!("as `{}`", s.local)
                    } else {
                        String::new()
                    }
                })
                .collect();
            (!hits.is_empty()).then_some((imp.file, imp.line, hits))
        })
        .collect();
    if importers.is_empty() {
        writeln!(
            out,
            "  imported by: nobody (entry point, framework-loaded, or unused)\n"
        )?;
    } else {
        writeln!(out, "  imported by:")?;
        for (f, line, hits) in importers {
            let extra = hits
                .into_iter()
                .filter(|h| !h.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            let extra = if extra.is_empty() {
                String::new()
            } else {
                format!("  ({extra})")
            };
            writeln!(out, "    {f}:{line}{extra}")?;
        }
        writeln!(out)?;
    }
    Ok(())
}
