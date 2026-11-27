# Architecture overview

WoW Tools (crate `wow-tools`) is a Windows tray utility and CLI in Rust for World of Warcraft: zip snapshots of `WTF`
and `Interface/AddOns` with restore, copying settings between characters, UI profiles, addon updates through the
installed addon managers (CurseForge, WowUp, Wago App) plus ElvUI, cleanup, and self-update from GitHub releases.

- `src/` — the program (shell) over the core crate `crates/core` (see "Core and shells"). `main.rs` (CLI, clap) and `tray.rs` (tray, default with no subcommand) share the operations in
  `app.rs` (core); `watch.rs` (core) runs the automatic loop; UI windows (`panel.rs`, `menu.rs`, `settings.rs`) draw with
  Direct2D/DirectWrite through `gfx.rs`. Tests are inline `#[cfg(test)]` modules.
- `macros.rs` — the macro check (`wow-tools macros`): parser of `macros-cache.txt`, offline checks; tables in `crates/core/data/*.tsv`.
- `macro_names.rs` — spell/item name cache from wago.tools (`macros --update-names`, on request only) and the name check (wrong language, unknown).
- `macro_fix.rs` — the fixed text of a macro (pure, no I/O): per client language of the targets it rewrites wrong-language
  names (`Names::retarget`, byte spans from `macro_names::name_spans`), aliases (to the English command) and removed
  functions marked in `crates/core/data/deprecated-api.tsv` (`drop-in`; `first:<field>:<min>-<max>` table functions where only the first value is used; `one-arg`), recording the replaced byte ranges (`Variant::marks`); `macros::walk` attaches the result to every `Finding` of the
  macro (`Finding::fixed`), `report.rs` shows it on the first card with a Copy button (`Action::CopyFix`) and
  `macros::report` prints it. Read-only: the owner pastes the text into the game.
- `macro_store.rs` — the last macro check as `data/macro-report.json` (versioned, temp file + rename), loaded when the tray starts;
  the report window and the Macros tab show it with the check time, a new check overwrites it. `report.rs` draws text through
  `gfx::rich_text` (bold highlighted ranges) and supports mouse selection (`gfx::hit_index` / `range_rects`) and Ctrl+C.
- The panel's **Macros** tab (`panel.rs`, `tray::MacroCheck`): the check runs on its own thread on a button
  (`tray::scan_macros`); the name tables are loaded inside `macros::scan` per game version and dropped with it, so the
  tray keeps only the findings. The name download is a worker request (`Request::UpdateNames`, `Work::Names`).
  The panel never widens: "Open report" opens `report.rs`, a regular resizable window with every finding in full,
  sorted by `macros::Level` (red, yellow, grey), its size and place remembered.
- `crates/core/locales/` — interface strings in five languages (ru, en, de, fr, es), embedded by `i18n.rs`.
- `packaging/` — the Inno Setup installer, the portable marker and the winget manifests.
- `site/` — the landing page (`site/build.py` renders it per language), published by GitHub Pages.
- `.github/workflows/` — CI (fmt, clippy, tests, build, installer), releases, winget submission, Pages.

The module-by-module description is in the root `CLAUDE.md` ("Layout").

## Core and shells

The code is a cargo workspace (decision: `docs/adr/0002-core-crate-and-shells.md`). The root package `wow-tools`
(`src/`) is the shell; `crates/core` is the library `wow-tools-core`. The core must never import the UI, the tray,
the self-update, `schedule`, `diag` or the installer; it has no `clap` unless the `clap` feature is on (the program turns
it on for `Kind` and `chars::Part`). The program re-exports the core's modules at its root (`main.rs`), so its window
code keeps writing `crate::config` and `tr!`.

### Core modules (`crates/core/src`) and their main entry points

- `i18n` (+ `locales/`): `tr!`, `set`, `current`, `Lang`; `unknown_keys` / `russian_in_code` are source-scanning helpers
  for the tests of both crates.
- `config`: `Config` (load/save, `schedule_due`), `default_config_path`, `data_dir`, `portable_dir`, `migrate_legacy`.
- `wow`: `detect_root`, `resolve_root`, `all_flavors`, `Flavor`, `running_games`, `running_in`.
- `snapshot`: `Store` (`backup`, `list`, `restore`), `Kind`, `Outcome`, `select`, `progress` / `on_progress`.
- `app`: `Ctx` (`load`, `flavors`, `single_flavor`), `backup_all`, `restore_plan`, `restore_apply`, `restore_point`,
  `update_addons`, `safety_snapshot`, `ensure_game_closed`.
- `chars`: `list`, `find`, `copy`, `Part`. `lua`, `editmode`: SavedVariables and Edit Mode file editing.
- `profiles`: `save`, `apply`, `list`, `delete`, `characters`.
- `maintenance`: `scan`, `clean`, `check_patches`, `outdated`, `Item`.
- `watch`: `run` (the automatic loop; `Request` in, `Event` out).
- `updater`, `curseforge`, `wowup`, `wago`, `elvui`: addon updates (`updater::AddonApp`, `elvui::update`).
- `macros`, `macro_fix`, `macro_names`, `macro_store`: the macro check (`macros::scan`, `report`; `macro_store::load` /
  `save` for `data\macro-report.json`).
- `http`: `get` over WinHTTP and `set_user_agent` (the program sets `wow-tools/<version>` once at start; the default
  is `wow-tools`). `version`: `newer`.
- `lock`: see below.

### The WoW folder lock

`lock::acquire(&Flavor, what) -> Result<Guard>` locks `%LOCALAPPDATA%\WoW Tools\locks\<key>.lock` with
`File::try_lock`, where `<key>` is the first 16 hex digits of the SHA-256 of the lowercased flavor folder path
(the folder with `WTF` and `Interface`). The path ignores portable mode, so a portable and an installed copy exclude
each other, and flavors in different roots do not block each other. It is fail-fast: a held lock is the error
`app.busy` ("another WoW Tools is already working on {flavor}: {what}"); the automatic loops retry on their usual
schedule. Windows frees the lock when the process dies. A locked file cannot be read by another handle, so the holder
writes `<pid> <operation>` to `<key>.info`, read only for that message.

Taken at the outermost entry point only (a second handle in the same process would fail), until the guard drops:
`app::backup_all` (per flavor, a busy flavor becomes a report error), `app::restore_apply` (so also `restore_point`),
`app::update_addons` (every selected flavor, for the whole run), `profiles::apply`, `maintenance::clean` (all items),
the snapshot in `maintenance::check_patches` and the program's `copy-char`. `safety_snapshot` and `Store::backup` never
take it. A shell that cannot call the core (CLI) may take the same lock itself by the rule above.

### Shells

A Rust shell links the library. A shell in another language runs the `wow-tools` CLI with `--json` and reads one JSON object
per call (`src/cli_json.rs`; the contract is `docs/cli-json.md`, `docs/adr/0003-cli-json-contract.md`; the error codes come from
`errors::classify` in the core, so a Rust shell gets the same ones). `--json watch` is the exception that streams: one JSON line per `watch::Event` and per snapshot percent until stdin closes (`cli_json::watch`, `docs/adr/0004-watch-json-stream.md`). The tray's single-instance mutex (`Local\wow-tools-tray`) is separate and stays in the program.
