# Drift — torrent client (working name)

Fast, light, modern desktop BitTorrent client. Windows first.
Read this file first, then the doc for your task. Do not skim.

## Docs (source of truth, keep in sync with code)
| Doc | Read when |
|---|---|
| `docs/ARCHITECTURE.md` | touching module boundaries, contracts, IPC, or adding a dependency |
| `docs/FEATURES.md` | deciding scope; a feature not listed there needs approval |
| `docs/UI_SPEC.md` | any frontend work (tokens, layout, states, copy) |
| `docs/ROADMAP.md` | picking up work; tasks have IDs and acceptance criteria |
| `docs/PLUGINS.md` | anything about plugins, search providers, or independently updatable modules |
| `docs/DECISIONS.md` | before questioning a stack/design choice; append new decisions |

## Stack (decided, see DECISIONS.md)
- Engine: Rust, `librqbit` (pinned exact version) behind our own trait.
- Shell: Tauri v2 (WebView2). Never Electron.
- Frontend: Svelte 5 + TypeScript + Vite, plain CSS with tokens (no UI kit).
- Tests: `cargo test`, Vitest, one Playwright smoke test per milestone.

## Hard rules
1. **No Python.** No scripts, no one-liners. Rust, TS/Node, or PowerShell.
2. **Engine isolation.** Only `crates/drift-core/src/engine/rqbit.rs` may `use librqbit`. `drift-core` must not depend on `tauri` or `axum`. Everything else talks to `trait TorrentEngine`. A grep for `librqbit` outside that file must return nothing.
3. **Frontend never knows about librqbit or Tauri internals.** It uses `src/lib/api/` only (typed wrapper over `invoke`/events).
4. **Composition root is `main.rs`.** No globals, no singletons, one owner per piece of state.
5. **Contracts change deliberately.** Edit `ARCHITECTURE.md` in the same commit as any trait/IPC change.
6. **Add by addition.** New feature = new module registered in the composition root, not edits scattered through existing ones.
7. **Light means light.** Idle RAM target < 80 MB, installer < 15 MB, cold start < 1 s. A new dependency needs a one-line justification in DECISIONS.md.
8. **No speculative abstraction.** Only the engine and the settings store are interfaces today. Extract more when a second implementation exists.
9. **Windows-safe paths and I/O.** Long paths, reserved names, `\\`/`/` handling, files locked by AV. Never assume Unix.

## Working rules
- Tests: new non-trivial logic (queue scheduler, ratio rules, size/ETA formatting, filters) gets a unit test against a fake engine. UI: no snapshot-test sprawl.
- UI changes are verified by running the app and **measuring** (real viewport, computed sizes, screenshots), never by reading CSS. Suspect stale cache if correct code looks broken.
- Review after non-trivial edits: Rust → `ecc:rust-reviewer` (else `ecc:code-reviewer`), Svelte/TS → `ecc:code-reviewer`. Use `model: "opus"` for reviewers.
- Do not add features, settings, or config for hypothetical needs. Ship the smallest thing that meets the acceptance criteria.
- Test torrents: only legal ones (Linux distro ISOs). Never commit downloaded data.
- Commits: small, one task ID each (`M2-04: …`). Update the task status in `docs/ROADMAP.md` in the same commit.

## Definition of done (every task)
Acceptance criteria met · tests pass · `cargo clippy -D warnings` and `npm run check` clean · docs updated · UI checked in the running app · task ticked in ROADMAP.
