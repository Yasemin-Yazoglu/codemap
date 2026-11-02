//! Next.js App Router conventions: routes, layouts, route handlers and the request interceptor
//! (`proxy.ts`, formerly `middleware.ts`). Pure functions over already-indexed files, so it is
//! testable without touching the disk.
use codemap_core::model::Directive;
use std::collections::BTreeMap;

/// The slice of an indexed file this analyzer needs.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: String,
    pub directive: Directive,
    pub exports: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpecialKind {
    Page,
    Layout,
    Route,
    Template,
    Loading,
    Error,
    NotFound,
}

impl SpecialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SpecialKind::Page => "page",
            SpecialKind::Layout => "layout",
            SpecialKind::Route => "route",
            SpecialKind::Template => "template",
            SpecialKind::Loading => "loading",
            SpecialKind::Error => "error",
            SpecialKind::NotFound => "not-found",
        }
    }

    fn from_stem(stem: &str) -> Option<Self> {
        Some(match stem {
            "page" => Self::Page,
            "layout" => Self::Layout,
            "route" => Self::Route,
            "template" => Self::Template,
            "loading" => Self::Loading,
            "error" => Self::Error,
            "not-found" => Self::NotFound,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteFile {
    pub kind: SpecialKind,
    pub path: String,
    pub directive: Directive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// e.g. `/receipts/:id`, `/docs/*slug`, `/shop/*slug?`
    pub url: String,
    /// The `page.*` or `route.*` file that serves this URL.
    pub entry: RouteFile,
    /// HTTP methods exported by a route handler (empty for pages).
    pub methods: Vec<String>,
    /// Layouts wrapping the entry, outermost first.
    pub layouts: Vec<RouteFile>,
    /// Nearest template/loading/error/not-found file that applies to the entry.
    pub boundaries: Vec<RouteFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterceptorKind {
    /// `proxy.ts` (Next.js 16+).
    Proxy,
    /// `middleware.ts` (older name).
    Middleware,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interceptor {
    pub kind: InterceptorKind,
    pub path: String,
}

#[derive(Debug, Clone, Default)]
pub struct NextApp {
    /// `app` or `src/app`, or `None` if the project has no App Router directory.
    pub app_root: Option<String>,
    pub routes: Vec<Route>,
    pub interceptor: Option<Interceptor>,
}

const HTTP_METHODS: &[&str] = &["GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE"];
const CODE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx"];

pub fn analyze(files: &[SourceFile]) -> NextApp {
    let app_root = if files.iter().any(|f| f.path.starts_with("src/app/")) {
        Some("src/app")
    } else if files.iter().any(|f| f.path.starts_with("app/")) {
        Some("app")
    } else {
        None
    };

    let mut app = NextApp {
        app_root: app_root.map(String::from),
        ..Default::default()
    };
    app.interceptor = find_interceptor(files);
    let Some(app_root) = app_root else { return app };

    // directory segments -> special files in that directory
    let mut dirs: BTreeMap<Vec<String>, Vec<(SpecialKind, &SourceFile)>> = BTreeMap::new();
    for f in files {
        let Some(rel) = f
            .path
            .strip_prefix(app_root)
            .and_then(|r| r.strip_prefix('/'))
        else {
            continue;
        };
        let mut parts: Vec<&str> = rel.split('/').collect();
        let Some(file_name) = parts.pop() else {
            continue;
        };
        let Some((stem, ext)) = file_name.rsplit_once('.') else {
            continue;
        };
        if !CODE_EXTENSIONS.contains(&ext) {
            continue;
        }
        let Some(kind) = SpecialKind::from_stem(stem) else {
            continue;
        };
        dirs.entry(parts.into_iter().map(String::from).collect())
            .or_default()
            .push((kind, f));
    }

    let route_file = |kind: SpecialKind, f: &SourceFile| RouteFile {
        kind,
        path: f.path.clone(),
        directive: f.directive,
    };

    for (segments, entries) in &dirs {
        let Some(url) = url_for(segments) else {
            continue;
        };
        let Some(&(kind, entry_file)) = entries
            .iter()
            .find(|(k, _)| matches!(k, SpecialKind::Page | SpecialKind::Route))
        else {
            continue;
        };

        // Route handlers return raw responses: layouts and UI boundaries only apply to pages.
        let is_page = kind == SpecialKind::Page;
        let mut layouts = Vec::new();
        for depth in (0..=segments.len()).filter(|_| is_page) {
            if let Some(&(_, f)) = dirs
                .get(&segments[..depth])
                .and_then(|es| es.iter().find(|(k, _)| *k == SpecialKind::Layout))
            {
                layouts.push(route_file(SpecialKind::Layout, f));
            }
        }

        let mut boundaries = Vec::new();
        for want in [
            SpecialKind::Template,
            SpecialKind::Loading,
            SpecialKind::Error,
            SpecialKind::NotFound,
        ]
        .into_iter()
        .filter(|_| is_page)
        {
            // nearest first: walk from the entry's directory up to the root
            for depth in (0..=segments.len()).rev() {
                if let Some(&(_, f)) = dirs
                    .get(&segments[..depth])
                    .and_then(|es| es.iter().find(|(k, _)| *k == want))
                {
                    boundaries.push(route_file(want, f));
                    break;
                }
            }
        }

        let methods = if kind == SpecialKind::Route {
            HTTP_METHODS
                .iter()
                .filter(|m| entry_file.exports.iter().any(|e| e == *m))
                .map(|m| m.to_string())
                .collect()
        } else {
            Vec::new()
        };

        app.routes.push(Route {
            url,
            entry: route_file(kind, entry_file),
            methods,
            layouts,
            boundaries,
        });
    }
    app.routes.sort_by(|a, b| a.url.cmp(&b.url));
    app
}

/// Directory segments -> URL. `None` for directories that are not directly routable
/// (private `_x`, parallel `@slot`, intercepting `(.)x` routes).
fn url_for(segments: &[String]) -> Option<String> {
    let mut url = String::new();
    for seg in segments {
        if seg.starts_with('_') || seg.starts_with('@') || seg.starts_with("(.") {
            return None;
        }
        if seg.starts_with('(') && seg.ends_with(')') {
            continue; // route group
        }
        let converted =
            if let Some(inner) = seg.strip_prefix("[[...").and_then(|s| s.strip_suffix("]]")) {
                format!("*{inner}?")
            } else if let Some(inner) = seg.strip_prefix("[...").and_then(|s| s.strip_suffix(']')) {
                format!("*{inner}")
            } else if let Some(inner) = seg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                format!(":{inner}")
            } else {
                seg.clone()
            };
        url.push('/');
        url.push_str(&converted);
    }
    Some(if url.is_empty() { "/".into() } else { url })
}

fn find_interceptor(files: &[SourceFile]) -> Option<Interceptor> {
    // `proxy` wins if both exist: it is the newer name.
    for (stem, kind) in [
        ("proxy", InterceptorKind::Proxy),
        ("middleware", InterceptorKind::Middleware),
    ] {
        for dir in ["", "src/"] {
            for ext in CODE_EXTENSIONS {
                let path = format!("{dir}{stem}.{ext}");
                if files.iter().any(|f| f.path == path) {
                    return Some(Interceptor { kind, path });
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str) -> SourceFile {
        SourceFile {
            path: path.into(),
            directive: Directive::None,
            exports: vec![],
        }
    }

    fn urls(app: &NextApp) -> Vec<&str> {
        app.routes.iter().map(|r| r.url.as_str()).collect()
    }

    #[test]
    fn url_conversion() {
        let app = analyze(&[
            f("app/page.tsx"),
            f("app/analysis/page.tsx"),
            f("app/(auth)/login/page.tsx"),
            f("app/receipts/[id]/page.tsx"),
            f("app/docs/[...slug]/page.tsx"),
            f("app/shop/[[...slug]]/page.tsx"),
        ]);
        assert_eq!(
            urls(&app),
            [
                "/",
                "/analysis",
                "/docs/*slug",
                "/login",
                "/receipts/:id",
                "/shop/*slug?"
            ]
        );
    }

    #[test]
    fn non_routable_directories_are_skipped() {
        let app = analyze(&[
            f("app/_components/page.tsx"),
            f("app/@modal/page.tsx"),
            f("app/feed/(.)photo/page.tsx"),
            f("app/ok/page.tsx"),
        ]);
        assert_eq!(urls(&app), ["/ok"]);
    }

    #[test]
    fn layouts_are_ordered_outermost_first_and_groups_contribute() {
        let app = analyze(&[
            f("app/layout.tsx"),
            f("app/(auth)/layout.tsx"),
            f("app/(auth)/login/page.tsx"),
            f("app/other/page.tsx"),
        ]);
        let login = app.routes.iter().find(|r| r.url == "/login").unwrap();
        let layouts: Vec<_> = login.layouts.iter().map(|l| l.path.as_str()).collect();
        assert_eq!(layouts, ["app/layout.tsx", "app/(auth)/layout.tsx"]);
        let other = app.routes.iter().find(|r| r.url == "/other").unwrap();
        assert_eq!(other.layouts.len(), 1);
    }

    #[test]
    fn nearest_boundary_wins() {
        let app = analyze(&[
            f("app/loading.tsx"),
            f("app/analysis/loading.tsx"),
            f("app/error.tsx"),
            f("app/analysis/page.tsx"),
        ]);
        let r = &app.routes[0];
        let b: Vec<_> = r.boundaries.iter().map(|b| b.path.as_str()).collect();
        assert_eq!(b, ["app/analysis/loading.tsx", "app/error.tsx"]);
    }

    #[test]
    fn route_handlers_have_no_layouts_or_boundaries() {
        let app = analyze(&[
            f("app/layout.tsx"),
            f("app/error.tsx"),
            f("app/api/x/route.ts"),
        ]);
        assert!(app.routes[0].layouts.is_empty());
        assert!(app.routes[0].boundaries.is_empty());
    }

    #[test]
    fn route_handler_methods() {
        let mut handler = f("app/api/receipts/route.ts");
        handler.exports = vec!["GET".into(), "POST".into(), "helper".into()];
        let app = analyze(&[handler]);
        assert_eq!(app.routes[0].entry.kind, SpecialKind::Route);
        assert_eq!(app.routes[0].methods, ["GET", "POST"]);
    }

    #[test]
    fn src_app_layout_and_interceptor_names() {
        let app = analyze(&[f("src/app/page.tsx"), f("src/middleware.ts")]);
        assert_eq!(app.app_root.as_deref(), Some("src/app"));
        assert_eq!(app.interceptor.unwrap().kind, InterceptorKind::Middleware);

        let app = analyze(&[f("proxy.ts"), f("middleware.ts")]);
        let i = app.interceptor.unwrap();
        assert_eq!(
            (i.kind, i.path.as_str()),
            (InterceptorKind::Proxy, "proxy.ts")
        );
    }

    #[test]
    fn no_app_dir() {
        let app = analyze(&[f("pages/index.tsx")]);
        assert!(app.app_root.is_none() && app.routes.is_empty());
    }
}
