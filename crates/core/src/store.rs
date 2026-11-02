use crate::model::*;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

const SCHEMA_VERSION: &str = "1";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS files (
    path TEXT PRIMARY KEY, hash TEXT NOT NULL, directive TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY,
    file TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    name TEXT NOT NULL, kind TEXT NOT NULL,
    exported INTEGER NOT NULL, is_default INTEGER NOT NULL,
    start_line INTEGER NOT NULL, end_line INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file);
CREATE TABLE IF NOT EXISTS imports (
    id INTEGER PRIMARY KEY,
    file TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    source TEXT NOT NULL, resolved TEXT, line INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS idx_imports_resolved ON imports(resolved);
CREATE TABLE IF NOT EXISTS import_specifiers (
    import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    imported TEXT NOT NULL, local TEXT NOT NULL);
";

const DROP_ALL: &str = "
DROP TABLE IF EXISTS import_specifiers; DROP TABLE IF EXISTS imports;
DROP TABLE IF EXISTS symbols; DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS meta;
";

/// A file that imports some other file, with the bindings it imports.
#[derive(Debug, Clone)]
pub struct Importer {
    pub file: String,
    pub line: u32,
    pub specifiers: Vec<Specifier>,
}

#[derive(Debug, Clone)]
pub struct FileSummary {
    pub path: String,
    pub directive: Directive,
    pub exports: Vec<String>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        // The index is derived data: on a schema change we rebuild instead of migrating.
        let version: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .ok();
        if version.as_deref().is_some_and(|v| v != SCHEMA_VERSION) {
            conn.execute_batch(DROP_ALL)?;
        }
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
            [SCHEMA_VERSION],
        )?;
        Ok(Self { conn })
    }

    pub fn file_hash(&self, path: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT hash FROM files WHERE path = ?1", [path], |r| {
                r.get(0)
            })
            .ok())
    }

    pub fn known_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files ORDER BY path")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn remove_file(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", [path])?;
        Ok(())
    }

    /// Replace everything stored for `f.path` atomically.
    pub fn upsert_file(&mut self, f: &FileIndex) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM files WHERE path = ?1", [&f.path])?;
        tx.execute(
            "INSERT INTO files(path, hash, directive) VALUES (?1, ?2, ?3)",
            params![f.path, f.hash, f.directive.as_str()],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO symbols(file, name, kind, exported, is_default, start_line, end_line)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for s in &f.symbols {
                stmt.execute(params![
                    f.path,
                    s.name,
                    s.kind.as_str(),
                    s.exported,
                    s.is_default,
                    s.start_line,
                    s.end_line
                ])?;
            }
        }
        for imp in &f.imports {
            tx.execute(
                "INSERT INTO imports(file, source, resolved, line) VALUES (?1, ?2, ?3, ?4)",
                params![f.path, imp.source, imp.resolved, imp.line],
            )?;
            let id = tx.last_insert_rowid();
            for sp in &imp.specifiers {
                tx.execute(
                    "INSERT INTO import_specifiers(import_id, imported, local) VALUES (?1, ?2, ?3)",
                    params![id, sp.imported, sp.local],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Symbols matching `name`: exact (case-sensitive) or, if `fuzzy`, substring (case-insensitive).
    pub fn find_symbols(&self, name: &str, fuzzy: bool) -> Result<Vec<(String, Symbol)>> {
        let sql = if fuzzy {
            "SELECT file, name, kind, exported, is_default, start_line, end_line FROM symbols
             WHERE instr(lower(name), lower(?1)) > 0 ORDER BY name, file, start_line"
        } else {
            "SELECT file, name, kind, exported, is_default, start_line, end_line FROM symbols
             WHERE name = ?1 ORDER BY file, start_line"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([name], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Symbol {
                    name: r.get(1)?,
                    kind: SymbolKind::parse(&r.get::<_, String>(2)?),
                    exported: r.get(3)?,
                    is_default: r.get(4)?,
                    start_line: r.get(5)?,
                    end_line: r.get(6)?,
                },
            ))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn directive_of(&self, path: &str) -> Result<Directive> {
        let d: Option<String> = self
            .conn
            .query_row("SELECT directive FROM files WHERE path = ?1", [path], |r| {
                r.get(0)
            })
            .ok();
        Ok(d.map(|s| Directive::parse(&s)).unwrap_or(Directive::None))
    }

    pub fn importers_of(&self, path: &str) -> Result<Vec<Importer>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file, line FROM imports WHERE resolved = ?1 ORDER BY file, line",
        )?;
        let heads: Vec<(i64, String, u32)> = stmt
            .query_map([path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut spec_stmt = self
            .conn
            .prepare("SELECT imported, local FROM import_specifiers WHERE import_id = ?1")?;
        let mut out = Vec::new();
        for (id, file, line) in heads {
            let specifiers = spec_stmt
                .query_map([id], |r| {
                    Ok(Specifier {
                        imported: r.get(0)?,
                        local: r.get(1)?,
                    })
                })?
                .collect::<std::result::Result<_, _>>()?;
            out.push(Importer {
                file,
                line,
                specifiers,
            });
        }
        Ok(out)
    }

    /// Every file with its directive and the names it exports (`default` included).
    pub fn file_summaries(&self) -> Result<Vec<FileSummary>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, directive FROM files ORDER BY path")?;
        let files: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut sym_stmt = self.conn.prepare(
            "SELECT name, is_default FROM symbols WHERE file = ?1 AND exported = 1 ORDER BY name",
        )?;
        let mut out = Vec::new();
        for (path, directive) in files {
            let mut exports = Vec::new();
            let rows = sym_stmt.query_map([&path], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
            })?;
            for row in rows {
                let (name, is_default) = row?;
                exports.push(name);
                if is_default {
                    exports.push("default".to_string());
                }
            }
            out.push(FileSummary {
                path,
                directive: Directive::parse(&directive),
                exports,
            });
        }
        Ok(out)
    }

    pub fn counts(&self) -> Result<(i64, i64, i64)> {
        let q = |t: &str| -> Result<i64> {
            Ok(self
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))?)
        };
        Ok((q("files")?, q("symbols")?, q("imports")?))
    }
}
