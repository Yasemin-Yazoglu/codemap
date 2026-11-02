use codemap_cli::{indexer, report};
use codemap_core::store::Store;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/mini-next")
}

fn indexed() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let rep = indexer::index_project(&fixture(), &mut store).unwrap();
    assert!(rep.failed.is_empty(), "{:?}", rep.failed);
    store
}

#[test]
fn routes_of_fixture_app() {
    let out = report::routes(&indexed()).unwrap();
    println!("{out}");
    assert!(out.contains("App Router: app/"));
    assert!(out.contains("proxy.ts (proxy)"));
    for url in [
        "/ ",
        "/analysis",
        "/api/receipts",
        "/login",
        "/receipts/:id",
    ] {
        assert!(out.contains(url), "missing {url} in:\n{out}");
    }
    assert!(out.contains("{GET, POST}"));
    assert!(out.contains("loading app/analysis/loading.tsx"));
}

#[test]
fn show_resolves_alias_imports_across_files() {
    let out = report::show(&indexed(), "buildMonthlyData").unwrap();
    println!("{out}");
    assert!(out.contains("function buildMonthlyData"));
    assert!(out.contains("lib/analysis.ts:5-"));
    assert!(out.contains("app/analysis/page.tsx:2"));
}

#[test]
fn show_marks_client_components_and_type_only_imports() {
    let store = indexed();
    let out = report::show(&store, "AnalysisChart").unwrap();
    assert!(out.contains("[client]"));
    assert!(out.contains("app/analysis/page.tsx:1"));
    // `import type { MonthlyTotal }` still counts as an importer of the type.
    let out = report::show(&store, "MonthlyTotal").unwrap();
    assert!(out.contains("components/AnalysisChart.tsx:5"), "{out}");
}

#[test]
fn reindexing_is_incremental_and_detects_removals() {
    let dir = tempfile_copy();
    let mut store = Store::open_in_memory().unwrap();
    let first = indexer::index_project(&dir, &mut store).unwrap();
    assert!(first.reindexed > 5);
    let second = indexer::index_project(&dir, &mut store).unwrap();
    assert_eq!(
        second.reindexed, 0,
        "nothing changed, nothing should be re-parsed"
    );

    std::fs::write(dir.join("lib/db.ts"), "export const x = 1;\n").unwrap();
    let third = indexer::index_project(&dir, &mut store).unwrap();
    assert_eq!(third.reindexed, 1);

    std::fs::remove_file(dir.join("components/CustomTooltip.tsx")).unwrap();
    let fourth = indexer::index_project(&dir, &mut store).unwrap();
    assert_eq!(fourth.removed, 1);
    assert!(report::show(&store, "CustomTooltip")
        .unwrap()
        .contains("No symbol"));
}

fn tempfile_copy() -> PathBuf {
    let dst = std::env::temp_dir().join(format!("codemap-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy_dir(&fixture(), &dst);
    dst
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}
