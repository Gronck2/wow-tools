# 0001. Development pipeline from the dev-pipeline plugin

Status: accepted (2026-11-21)

## Context

Work on WoW Tools is done with Claude Code agents. A shared pipeline (manager -> coder -> tester + code-reviewer ->
qa-mr) is maintained once in the `dev-pipeline` plugin (marketplace `Gronck2/claude-plugins`) instead of per-repo
agent copies; each repo has to state its own branches, gates and rules.

## Decision

- The pipeline comes from the `dev-pipeline` plugin; project specifics live in the `## Pipeline contract` section of
  `CLAUDE.md`, which points to the existing `CLAUDE.md` sections instead of repeating them.
- Branch model: `main` only — the base and the PR target. Task branches `task/NNNN-slug`, docs-only `docs/<slug>`.
- Merging: PRs are opened by the pipeline; only the owner merges them, by squash, and only with green CI. Direct
  pushes to main, version bumps, tags, release publishing and force-push happen only on the owner's request.
- Gates: `cargo fmt --check`, `cargo clippy --release --all-targets --locked -- -D warnings`,
  `cargo test --release --locked` for the Rust part; `site/build.py` for `site/`. No separate `cargo build` gate
  (it fails while the tray runs from `target\release`).
- Branch protection on main and delete-branch-on-merge are not decided yet and stay off.

## Consequences

- The plugin must be installed and enabled on the machine; sessions load its agents at start, so open sessions need
  a restart or `/reload-plugins` after it changes.
- Tasks and plans are recorded in `docs/tasks/` and `docs/plan/`, decisions in `docs/adr/`.
