use serde_json::Value;
use std::path::{Component, Path, PathBuf};

const PROBE_EXTENSIONS: &[&str] = &["tsx", "ts", "jsx", "js", "mjs", "cjs"];

/// Resolves import specifiers to project files, honouring `tsconfig.json` `paths`/`baseUrl`.
/// Anything that does not resolve to a file inside the project (npm packages, `next/...`) yields `None`.
pub struct Resolver {
    root: PathBuf,
    /// Directory that `paths` targets are relative to.
    base: PathBuf,
    base_url_set: bool,
    paths: Vec<(String, Vec<String>)>,
}

impl Resolver {
    pub fn new(root: &Path) -> Self {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut r = Resolver {
            base: root.clone(),
            root,
            base_url_set: false,
            paths: Vec::new(),
        };
        let Ok(text) = std::fs::read_to_string(r.root.join("tsconfig.json")) else {
            return r;
        };
        let Ok(json) = serde_json::from_str::<Value>(&clean_jsonc(&text)) else {
            return r;
        };
        let opts = &json["compilerOptions"];
        if let Some(b) = opts["baseUrl"].as_str() {
            r.base = normalize(&r.root.join(b));
            r.base_url_set = true;
        }
        if let Some(map) = opts["paths"].as_object() {
            for (pattern, targets) in map {
                let targets = targets
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                r.paths.push((pattern.clone(), targets));
            }
        }
        r
    }

    /// `from_file` is project-relative with forward slashes. Returns a project-relative path.
    pub fn resolve(&self, from_file: &str, spec: &str) -> Option<String> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == ".." {
            let dir = Path::new(from_file)
                .parent()
                .unwrap_or_else(|| Path::new(""));
            candidates.push(self.root.join(dir).join(spec));
        } else {
            for (pattern, targets) in &self.paths {
                let captured = match pattern.find('*') {
                    Some(star) => {
                        let (prefix, suffix) = (&pattern[..star], &pattern[star + 1..]);
                        if spec.len() >= prefix.len() + suffix.len()
                            && spec.starts_with(prefix)
                            && spec.ends_with(suffix)
                        {
                            Some(&spec[prefix.len()..spec.len() - suffix.len()])
                        } else {
                            None
                        }
                    }
                    None => (pattern == spec).then_some(""),
                };
                if let Some(captured) = captured {
                    for t in targets {
                        candidates.push(self.base.join(t.replace('*', captured)));
                    }
                }
            }
            if self.base_url_set {
                candidates.push(self.base.join(spec));
            }
        }
        candidates
            .into_iter()
            .find_map(|c| self.probe(&normalize(&c)))
            .and_then(|f| {
                f.strip_prefix(&self.root)
                    .ok()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
            })
    }

    fn probe(&self, base: &Path) -> Option<PathBuf> {
        if !base.starts_with(&self.root) {
            return None;
        }
        if base.is_file() {
            return Some(base.to_path_buf());
        }
        for ext in PROBE_EXTENSIONS {
            let p = PathBuf::from(format!("{}.{ext}", base.display()));
            if p.is_file() {
                return Some(p);
            }
        }
        for ext in PROBE_EXTENSIONS {
            let p = base.join(format!("index.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }
}

/// Lexical normalisation (`a/../b` -> `b`) without touching the filesystem.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// tsconfig.json is JSONC: strip comments and trailing commas so serde_json can read it.
fn clean_jsonc(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut in_str) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else {
            out.push(c);
        }
        i += 1;
    }
    // Trailing commas: drop a comma whose next non-space char closes an object/array.
    let chars: Vec<char> = out.chars().collect();
    let mut result = String::with_capacity(out.len());
    let mut in_str = false;
    for (i, &c) in chars.iter().enumerate() {
        if in_str {
            result.push(c);
            if c == '"' && chars[i - 1] != '\\' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
        } else if c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        result.push(c);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::create_dir_all(p.join("lib")).unwrap();
        fs::create_dir_all(p.join("components/ui")).unwrap();
        fs::write(p.join("lib/db.ts"), "").unwrap();
        fs::write(p.join("components/ui/index.tsx"), "").unwrap();
        fs::write(p.join("components/Chart.tsx"), "").unwrap();
        fs::write(
            p.join("tsconfig.json"),
            r#"{ // comment
              "compilerOptions": { /* block */
                "paths": { "@/*": ["./*"], },
              },
            }"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn alias_relative_and_index_resolution() {
        let dir = project();
        let r = Resolver::new(dir.path());
        assert_eq!(
            r.resolve("app/page.tsx", "@/lib/db").as_deref(),
            Some("lib/db.ts")
        );
        assert_eq!(
            r.resolve("app/page.tsx", "@/components/ui").as_deref(),
            Some("components/ui/index.tsx")
        );
        assert_eq!(
            r.resolve("components/Chart.tsx", "../lib/db").as_deref(),
            Some("lib/db.ts")
        );
        assert_eq!(
            r.resolve("components/ui/index.tsx", "../Chart").as_deref(),
            Some("components/Chart.tsx")
        );
        assert_eq!(
            r.resolve("components/Chart.tsx", "./ui").as_deref(),
            Some("components/ui/index.tsx")
        );
    }

    #[test]
    fn external_and_missing_do_not_resolve() {
        let dir = project();
        let r = Resolver::new(dir.path());
        assert_eq!(r.resolve("app/page.tsx", "react"), None);
        assert_eq!(r.resolve("app/page.tsx", "recharts"), None);
        assert_eq!(r.resolve("app/page.tsx", "@/lib/nope"), None);
        assert_eq!(r.resolve("app/page.tsx", "../../../../etc/passwd"), None);
    }

    #[test]
    fn works_without_tsconfig() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.ts"), "").unwrap();
        let r = Resolver::new(dir.path());
        assert_eq!(r.resolve("b.ts", "./a").as_deref(), Some("a.ts"));
        assert_eq!(r.resolve("b.ts", "@/a"), None);
    }
}
