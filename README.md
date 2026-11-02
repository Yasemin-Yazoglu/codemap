# codemap

A local tool that helps you understand your own Next.js (App Router) codebase:
where things are defined, who uses them, and how requests flow through the app.

**Design rule:** facts come from deterministic analysis (tree-sitter + framework
rules), never from an LLM. A language model will later be used only to *explain*
those facts, so it can't hallucinate who calls what.

## Status: milestone 1

```
codemap [-r <project root>] index            # parse + update .codemap/index.db
codemap [-r <project root>] routes           # routes, layouts, route-handler methods
codemap [-r <project root>] show <name>      # definition + which files import it
```

Queries re-index automatically; unchanged files are skipped by content hash.

What it understands today:

- TS/TSX/JS/JSX parsing (tree-sitter), tolerant of syntax errors
- Top-level functions, components, hooks, classes, types, variables; `"use client"` / `"use server"`
- Imports and re-exports, resolved through `tsconfig.json` `paths` / `baseUrl` (JSONC supported)
- App Router routes: dynamic / catch-all segments, route groups, nearest layouts and
  `loading` / `error` / `not-found` files, route-handler HTTP methods
- `proxy.ts` (and legacy `middleware.ts`) detection

## Known limitations (planned next)

- "Imported by" is **file-level**, not call-level. A real call graph needs semantic resolution (SCIP / the TS compiler).
- `Component` is a naming heuristic (capitalised function); JSX is not inspected yet.
- Only top-level declarations; inline `"use server"` actions are not detected.
- `proxy.ts` matcher and auth-check analysis are not implemented.
- Pages Router (`pages/`) is not supported.
- No JSX / prop edges yet, so "what renders the chart tooltip?" is not answerable yet.

## Roadmap

1. **M2: JSX and props.** Render tree, prop edges, a Recharts knowledge table
   (`XAxis dataKey`, `Tooltip content`, ...), `codemap flow <route>`.
2. **M3: Understanding.** LLM-written descriptions with source citations, stale-flagging by hash,
   natural-language questions over graph + embeddings.
3. **M4: Security rules** (server/client leaks, unprotected actions, env exposure) and an editor extension.
4. **Later, optional:** link AI prompts to the changes they produced.

## Development

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Layout: `crates/core` (model + SQLite store), `crates/parse` (tree-sitter + resolver),
`crates/nextjs` (framework rules, pure functions), `crates/cli`; `fixtures/mini-next` is a
tiny app used by the integration tests.
