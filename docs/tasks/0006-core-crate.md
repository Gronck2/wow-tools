# 0006 Split the core from the window (cargo workspace)

## Goal
Move everything that is not window, tray, installer or self-update into a library crate `wow-tools-core`, and make the
existing package `wow-tools` (exe, CLI, tray, self-update) depend on it. Behaviour does not change: same files in
`%APPDATA%` / `%LOCALAPPDATA%`, same `data\macro-report.json`, same CLI arguments, same `target\release\wow-tools.exe`.
Add one thing the split needs: a lock so two processes never change the same WoW folder at once.

## Why
A second shell may appear next to the Rust one (Overwolf / ow-electron, own UI, store and updates), both on one core.
Only the core extraction now; nothing Overwolf or Electron.

## Decisions (made here, no owner input needed)
- **Layout:** a workspace whose root is still the package `wow-tools` (`[workspace] members = ["crates/core"]` added to
  the root `Cargo.toml`), core at `crates/core` (package `wow-tools-core`, lib `wow_tools_core`). A virtual root would move
  `src/`, `build.rs`, `packaging/` and break the paths in `ci.yml`, `release.yml`, `installer.iss`; this way
  `target\release\wow-tools.exe`, `build.rs`, `wow-tools.rc`, `Cargo.lock`, the `^version = ` check in the workflows
  and `--locked` stay as they are.
- **Core** (no Win32 UI, no clap): `i18n` (+ `locales/`), `config`, `wow`, `snapshot`, `chars`, `lua`, `editmode`,
  `profiles`, `maintenance`, `app`, `watch`, `updater`, `curseforge`, `wowup`, `wago`, `elvui`, `macros`, `macro_fix`,
  `macro_names`, `macro_store` (+ `data/*.tsv`), and two pieces cut out of `selfupdate.rs`: `http` (the WinHTTP `get`) and
  `newer` (version compare).
- **App** (stays in `src/`): `main` (clap, printing, `ui-snapshot`), `tray`, `panel`, `menu`, `settings`, `report`, `gfx`,
  `selfupdate` (rest), `schedule` (schtasks / HKCU Run, installer integration), `diag` (reads the app's version,
  Windows version, updater state; a shell would have its own).
- **Non-UI Win32 stays in core where it already is:** `winreg` in `wow.rs` (install detection), `windows-sys`
  Globalization in `i18n` (Windows UI language), `updater::hide_windows` (WinEvent hook that hides a manager's window),
  WinHTTP. They are not windows of ours, and the core is Windows-only anyway (the code already has non-Windows stubs; keep them).
- **Lock:** see Lock below. The tray's single-instance mutex (`tray.rs:376`, `Local\wow-tools-tray`) does not fit:
  it is per session and per tray, keyed by nothing about the WoW folder, and the CLI, `watch` and the scheduled task do not
  take it. It stays as is (app).
- **Shell API recommendation:** a second (Node/Electron) shell calls the **CLI with JSON output**, not a library:
  the CLI is already the stable contract, a process boundary keeps the lock and crashes contained, and no native
  addon (napi) build is needed. The Rust lib is for Rust consumers (our own exe). The JSON output itself is a later task.

## Scope
1. Workspace + `crates/core`; modules moved with `git mv` (history kept), paths and `use`s adjusted, no renames.
2. `tr!` and `i18n` in core (core strings already use `tr!`); locales and `data/` move into `crates/core/`.
3. `selfupdate::http` and `newer` into core; `elvui` and `macro_names` use the core ones; `selfupdate` re-uses them.
4. The sync lock (new `crates/core/src/lock.rs`) taken by the mutating entry points, one new locale string x 5 languages.
5. Tests: core tests run with `cargo test -p wow-tools-core` (no window code is compiled); CI and gates use `--workspace`.
6. `docs/arch/overview.md` (core vs app, the lock, the shell API recommendation), `CLAUDE.md` Layout paths and gate
   commands, `THIRD-PARTY.md` path of `realms.tsv`.

## Lock
- A file lock (`std::fs::File::try_lock`, stable since Rust 1.89: no new `unsafe`, no dependency) on
  `%LOCALAPPDATA%\WoW Tools\locks\<sha256(lowercased flavor folder)[..16]>.lock`. Windows releases it when the process dies,
  so a crash leaves nothing stale. The path ignores portable mode on purpose: a portable and an installed copy on the same
  WoW folder must exclude each other.
- Key = the flavor folder (where WTF and AddOns live), the same "WoW folder" a restore or a copy touches; flavors of other
  roots (Battle.net installs elsewhere) do not block each other.
- Taken at the outermost entry only, never nested (a second handle in the same process would fail): `app::backup_all`
  (per flavor inside its loop), `app::restore_apply`, `app::update_addons` (per selected flavor, held for the run),
  `profiles::apply`, `maintenance::clean` (every item that touches WTF/AddOns; Cache and Logs included, simpler and
  harmless), `maintenance::check_patches` snapshot, `main::cmd_copy_char` (non dry-run). Callees such as
  `safety_snapshot`, `Store::backup`, `restore_point` (calls `restore_apply`) do not take it.
- Fail-fast, not blocking. Busy = an error with a new string `app.busy` ("another WoW Tools is working on {flavor}:
  {what}") in ru/en/de/fr/es. The automatic loops already retry a failed round after 10 min; a manual action shows the error.
  The lock file holds the holder's pid and operation name, only for that message.
- Honoured by the future shell by calling the core (CLI) rather than by re-implementing it; the file location and the
  hash rule are documented in `docs/arch/overview.md` so a shell can take the same lock itself if it ever must.

## Out of scope
Any Overwolf/Electron code, JSON output of the CLI, napi bindings, moving `schedule`/`diag` to core, renaming anything
(types, functions, files, the exe), version bump, changing what an operation does, publishing core to crates.io,
changing config format or paths.

## Acceptance
- `cargo fmt --all --check`, `cargo clippy --workspace --release --all-targets --locked -- -D warnings`,
  `cargo test --workspace --release --locked` pass; `cargo test -p wow-tools-core --release --locked` passes alone.
- `cargo tree -p wow-tools-core` has no `clap`; core sources contain no `crate::` reference to an app module.
- Every i18n test still passes (all languages have all keys, every `tr!` key exists, no Russian in non-test code) and
  still scans the sources of **both** crates.
- `wow-tools ui-snapshot ... ` output unchanged for a few views (compare PNG bytes before/after on the same demo data).
- Read-only CLI against a scratch config (`status`, `chars`, `list`, `macros`) prints the same as before.
- CI (`ci.yml`) and the release workflow build the same artifacts at the same paths; installer step unchanged.
- Lock tests: second acquire of the same folder fails with the `app.busy` text, a different folder succeeds, a dropped
  guard frees it; a `backup_all` run against a held folder reports the error and touches nothing.

## Open questions (decided by the owner)
1. Fail-fast vs wait: **decided: fail-fast** (new `app.busy` string in all five locales, Russian first); the 10-min retry
   of the automatic loops stays. Waiting with a timeout is a small later change inside `lock.rs`.
2. `data/` and `locales/`: **decided: move into `crates/core/`**; the READMEs and the site keep naming `locales/*.toml` and
   `site/locales/` in prose (checked in step 7).
3. ADR: **decided: write** `docs/adr/0002-core-crate-and-shells.md` (core + shells split; CLI with JSON for a non-Rust
   shell such as Electron/Overwolf, the library for Rust consumers) and update `docs/arch/overview.md` with the core's
   public API and the lock.

## Risks
- `include_str!` paths (`i18n.rs`, `macros.rs`) and the i18n tests that read `CARGO_MANIFEST_DIR/src`.
- `tr!` is `macro_rules!` with `$crate::i18n::..`: needs `#[macro_export]`, and `#[macro_use] extern crate wow_tools_core;`
  in the app keeps the unqualified `tr!` everywhere.
- `selfupdate::http` builds its user agent from the app version (`wow-tools/<ver>`); core must not take the core crate's
  version: the app sets it once at start (`http::set_user_agent`).
- `CARGO_PKG_*` and `env!` uses remain in app files only (checked: `diag`, `menu`, `selfupdate`, `settings`).
- `cargo test` at the root no longer covers core: every gate and both workflows must say `--workspace`.
- `build.rs` / `wow-tools.rc` stay with the root package only; `.cargo/config.toml` (static CRT) applies to the workspace.
- `Cargo.lock` is shared; dependencies move between manifests (`chrono`, `serde`, `zip`, `walkdir`, `sha2`, `sysinfo`,
  `winreg`, `windows-sys` features) and the lock must not change other than adding the crate (`--locked` proves it).
