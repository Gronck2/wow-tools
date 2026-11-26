# Plan 0006

Mechanical move in small commits; the tree builds and tests pass after each step. Use `git mv`.

## Dependency facts (grep of `crate::`)
- Core modules depend only on each other, except: `elvui` and `macro_names` -> `selfupdate::{http::get, newer}`;
  `config` -> `updater::AddonApp` and `i18n::Lang` (both core). `diag`, `selfupdate`, `schedule` and all windows are
  leaves toward the app. `watch` uses only `app, chars, config, maintenance, profiles, snapshot, macro_names` (core).
- `macro_store`, `macro_fix`, `macro_names`, `macros` depend on `config`, `wow`, `chars`, `profiles` only; `report.rs`
  and `tray.rs` use them, not the other way round.

## Steps
1. **Cut the two pieces out of `selfupdate.rs`** (still one crate): `http` module (both `cfg(windows)` and the
   non-Windows stub) -> `src/http.rs`; `newer` -> same file or `src/version.rs`; user agent from a `OnceLock<String>`
   set by `selfupdate`/`main` (`http::set_user_agent("wow-tools/<ver>")` at start of `main`), default `"wow-tools"`.
   `selfupdate` does `pub use crate::http; pub use crate::version::newer;` so its callers do not change. Tests pass.
2. **Add the lock** (still one crate, `src/lock.rs`): `Lock::acquire(&Flavor, what: &str) -> Result<Guard>`; path and hash
   as in the task; the guard unlocks on drop; writes `pid op` into the file for the message; `tr!("app.busy", ...)` added
   to the five locale files (ru first, paths in single quotes). Take it in the entry points listed in the task. Unit
   tests with `tempfile` (override the lock dir through an argument or a `cfg(test)` base; never the real one).
   Reason for doing it before the split: behaviour change and move are reviewable separately.
3. **Create the workspace:** root `Cargo.toml` gets `[workspace] members = ["crates/core"]`; `crates/core/Cargo.toml`
   (`name = "wow-tools-core"`, edition 2024, `license`, `repository`, `version = "0.0.0"`, `publish = false`) with the
   dependencies the core modules use: `anyhow, chrono, directories, log, serde, serde_json (preserve_order +
   arbitrary_precision), sha2, sysinfo, toml, walkdir, zip`, `[dev-dependencies] tempfile`, windows: `winreg`,
   `windows-sys` (Foundation, Globalization, Networking_WinHttp, System_Threading, UI_Accessibility,
   UI_WindowsAndMessaging, plus what `updater`/`wow` use — compile will tell). Root keeps clap, env_logger, windows,
   windows-numerics, the UI windows-sys features, and adds `wow-tools-core = { path = "crates/core" }`.
   Features that the root needs from shared crates (serde_json features) must be identical so the resolver unifies.
4. **Move the core modules** with `git mv src/X.rs crates/core/src/X.rs` for the list in the task, plus `locales/` and
   `data/` to `crates/core/`. Create `crates/core/src/lib.rs` with `pub mod` lines; `tr!` gets `#[macro_export]` (it is
   defined in `i18n.rs`, `$crate` then points to core). Fix `include_str!` (`../locales/..` is still right relative to
   `crates/core/src/i18n.rs` after moving `locales/` next to `src/`, same for `data/`). Items used by the app become
   `pub`; do not widen more than the compiler asks (`pub(crate)` -> `pub` where used from the app).
5. **App side:** in `main.rs` replace the `mod` lines of moved modules with
   `#[macro_use] extern crate wow_tools_core; pub(crate) use wow_tools_core::{app, chars, config, editmode, elvui, i18n, lua, macro_fix, macro_names, macro_store, macros, maintenance, profiles, snapshot, updater, wago, watch, wow, ...};`
   so every `crate::config::..` in the window modules keeps working untouched (smallest diff). Keep `selfupdate`,
   `schedule`, `diag`, `gfx`, `panel`, `menu`, `report`, `settings`, `tray` as modules. `selfupdate`'s own `http`
   re-export points at `wow_tools_core::http`.
6. **Tests:**
   - core: all moved inline tests run under `cargo test -p wow-tools-core`. The i18n tests that scan sources (`every tr!
     key exists`, `no Russian in non-test code`) stay in core for core's `src`, and a twin test in the app scans app's
     `src` (reuse a `pub fn` helper from core `i18n` taking a dir, `#[doc(hidden)]`, to avoid copying the code).
     The key-parity test (every language has every key, same `{args}`) stays in core only.
   - app: tests of `panel`, `tray`, `report`, `settings`, `main` unchanged. `main.rs` tests that touch moved code only via
     `use`.
   - The tests assert Russian text and need `i18n::current()` = Russian: check it still holds without a tray (it did before).
7. **Workflows and gates:** `ci.yml` and `release.yml`: `cargo fmt --all --check`, `cargo clippy --workspace --release
   --all-targets --locked -- -D warnings`, `cargo test --workspace --release --locked`; `cargo build --release --locked`
   stays (default member = root package, builds core as a dependency; exe path unchanged). The version regex on
   `Cargo.toml` keeps working (root package stays `version = "0.8.0"` literal; do not use `workspace.package` for it).
   Update `CLAUDE.md`: Layout paths (`crates/core/src/..`), the "Build and check" and Pipeline contract gate commands,
   `locales/` / `data/` paths; `THIRD-PARTY.md` (`crates/core/data/realms.tsv`). READMEs mention `locales/` only in
   prose about the repo: check with grep, they must stay in step in all five languages.
8. **Docs:** `docs/arch/overview.md` gets a "Core and shells" section: crate map, what the core must never import (UI,
   tray, self-update, installer), the lock (path, key, who takes it, fail-fast), the `http::set_user_agent` hook, and the
   recommendation: another shell uses the CLI with JSON output (to be built in its own task), a Rust shell the library.
9. **Verify** (PATH from CLAUDE.md "Build and check"): the gates; `cargo tree -p wow-tools-core | grep -i clap` is empty;
   `grep -rn "crate::\(tray\|panel\|gfx\|menu\|settings\|report\|schedule\|selfupdate\|diag\)" crates/core/src` is empty;
   `ui-snapshot` of `panel`, `settings`, `macro-report` in ru and de before/after: identical PNG bytes; scratch-config run of
   `status`, `chars`, `list`, `macros`, `backup` (scratch `backup_dir`) identical text, and a second parallel `backup`
   prints the busy error. The `site/` build is untouched (no site files change; run it only if `docs/images` moves: it does not).
   Do not stop or replace the running tray: `cargo test` and `clippy` do not write `target\release\wow-tools.exe`.

## Files likely touched
`Cargo.toml`, `Cargo.lock`, `crates/core/**` (moved sources, `Cargo.toml`, `lib.rs`, `lock.rs`, `locales/`, `data/`),
`src/main.rs`, `src/selfupdate.rs`, `src/elvui.rs` and `src/macro_names.rs` (now in core, import `crate::http`), `src/app.rs`,
`src/profiles.rs`, `src/maintenance.rs` (lock calls), `.github/workflows/ci.yml`, `.github/workflows/release.yml`,
`CLAUDE.md`, `THIRD-PARTY.md`, `docs/arch/overview.md`, `CHANGELOG.md` only if the lock message counts as user-visible
(it does: one line under `[Unreleased]`).

## Tradeoffs considered
- Virtual workspace root (`crates/app` + `crates/core`): cleaner, but moves `src/`, `build.rs`, `packaging` references and the
  exe path in CI and the installer; no gain for a first split. Revisit when a second app crate appears.
- Re-exporting the core modules at the app root (`pub(crate) use wow_tools_core::..`) vs rewriting every `crate::x` to
  `wow_tools_core::x`: the re-export keeps the diff small and mechanical; a later cleanup can drop it.
- Named mutex (`Local\wow-tools-sync-<hash>`) vs file lock: mutex ownership is per thread (the tray worker, `watch` and
  the update thread differ) and needs unsafe Win32; the file lock is portable, per handle, and visible on disk for
  diagnostics. Global namespace was rejected: it needs privileges across sessions and nothing here spans sessions.
- Lock inside `Store::backup` (finest) rejected: `safety_snapshot` and `update_addons` call it inside an action that
  already holds the lock, and a second handle in the same process would fail.
- `watch` to core: the loop has no Win32 and a second shell (headless) may want it through the CLI. `tray` keeps its
  own use of `watch::Request/Event`.
- `schedule` and `diag` in core: rejected (they know the exe path, installer, HKCU Run key and the app version).
