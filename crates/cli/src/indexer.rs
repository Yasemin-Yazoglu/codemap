use anyhow::{Context, Result};
use codemap_core::store::Store;
use codemap_parse::{content_hash, extract, Resolver, SOURCE_EXTENSIONS};
use ignore::WalkBuilder;
use std::collections::BTreeSet;
use std::path::Path;

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".next",
    ".git",
    "target",
    ".codemap",
    "dist",
    "build",
    "out",
];

#[derive(Debug, Default)]
pub struct IndexReport {
    pub scanned: usize,
    pub reindexed: usize,
    pub removed: usize,
    pub failed: Vec<(String, String)>,
}

/// Project-relative paths (forward slashes) of all source files, honouring `.gitignore`.
pub fn collect_sources(root: &Path) -> Result<Vec<String>> {
    let mut out = BTreeSet::new();
    let walker = WalkBuilder::new(root)
        .require_git(false)
        .filter_entry(|e| {
            let skip = e.file_type().is_some_and(|t| t.is_dir())
                && e.file_name()
                    .to_str()
                    .is_some_and(|n| SKIP_DIRS.contains(&n));
            !skip
        })
        .build();
    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !SOURCE_EXTENSIONS.contains(&ext) || name.ends_with(".d.ts") {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(path);
        out.insert(rel.to_string_lossy().replace('\\', "/"));
    }
    Ok(out.into_iter().collect())
}

/// Bring `store` in line with the files on disk. Unchanged files (same content hash) are skipped.
pub fn index_project(root: &Path, store: &mut Store) -> Result<IndexReport> {
    let paths = collect_sources(root)?;
    let known: BTreeSet<String> = store.known_paths()?.into_iter().collect();
    let current: BTreeSet<&String> = paths.iter().collect();

    let removed: Vec<&String> = known.iter().filter(|p| !current.contains(p)).collect();
    let added = paths.iter().any(|p| !known.contains(p));
    // Import resolution depends on which files exist, so any added/removed file
    // can change how *other* files' imports resolve. Re-extract everything then.
    let force = added || !removed.is_empty();

    let resolver = Resolver::new(root);
    let mut report = IndexReport {
        scanned: paths.len(),
        ..Default::default()
    };

    for rel in &paths {
        let source = match std::fs::read_to_string(root.join(rel)) {
            Ok(s) => s,
            Err(e) => {
                report.failed.push((rel.clone(), e.to_string()));
                continue;
            }
        };
        if !force && store.file_hash(rel)?.as_deref() == Some(content_hash(&source).as_str()) {
            continue;
        }
        match extract(rel, &source) {
            Ok(mut file) => {
                for imp in &mut file.imports {
                    imp.resolved = resolver.resolve(rel, &imp.source);
                }
                store
                    .upsert_file(&file)
                    .with_context(|| format!("storing {rel}"))?;
                report.reindexed += 1;
            }
            Err(e) => report.failed.push((rel.clone(), e.to_string())),
        }
    }
    for p in removed {
        store.remove_file(p)?;
        report.removed += 1;
    }
    Ok(report)
}
