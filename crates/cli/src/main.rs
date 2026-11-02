use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use codemap_cli::{indexer, report};
use codemap_core::store::Store;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "codemap", version, about = "Understand your Next.js codebase")]
struct Cli {
    /// Project root (the directory containing package.json).
    #[arg(long, short, global = true, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse the project and update the index in `.codemap/`.
    Index,
    /// List App Router routes with their layouts and route-handler methods.
    Routes,
    /// Show where a function, component or variable is defined and who imports it.
    Show { name: String },
}

fn open_store(root: &std::path::Path) -> Result<Store> {
    let dir = root.join(".codemap");
    std::fs::create_dir_all(&dir)?;
    // Keep the index out of version control without touching the user's own .gitignore.
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(ignore, "*\n")?;
    }
    Store::open(&dir.join("index.db"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = cli.root.canonicalize()?;
    if !root.join("package.json").exists() {
        bail!(
            "{} has no package.json; pass the project root with --root",
            root.display()
        );
    }
    let mut store = open_store(&root)?;
    // Queries always see a fresh index; unchanged files are skipped by hash, so this is cheap.
    let rep = indexer::index_project(&root, &mut store)?;

    match cli.command {
        Command::Index => {
            let (files, symbols, imports) = store.counts()?;
            println!(
                "Indexed {} files ({} updated, {} removed): {symbols} symbols, {imports} imports",
                files, rep.reindexed, rep.removed
            );
            for (path, err) in &rep.failed {
                eprintln!("warning: {path}: {err}");
            }
        }
        Command::Routes => print!("{}", report::routes(&store)?),
        Command::Show { name } => print!("{}", report::show(&store, &name)?),
    }
    Ok(())
}
