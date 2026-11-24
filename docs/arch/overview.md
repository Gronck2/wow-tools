# Architecture overview

WoW Tools (crate `wow-tools`) is a Windows tray utility and CLI in Rust for World of Warcraft: zip snapshots of `WTF`
and `Interface/AddOns` with restore, copying settings between characters, UI profiles, addon updates through the
installed addon managers (CurseForge, WowUp, Wago App) plus ElvUI, cleanup, and self-update from GitHub releases.

- `src/` — one binary. `main.rs` (CLI, clap) and `tray.rs` (tray, default with no subcommand) share the operations in
  `app.rs`; `watch.rs` runs the automatic loop; UI windows (`panel.rs`, `menu.rs`, `settings.rs`) draw with
  Direct2D/DirectWrite through `gfx.rs`. Tests are inline `#[cfg(test)]` modules.
- `macros.rs` — the macro check (`wow-tools macros`): parser of `macros-cache.txt`, offline checks; tables in `data/*.tsv`.
- `macro_names.rs` — spell/item name cache from wago.tools (`macros --update-names`, on request only) and the name check (wrong language, unknown).
- The panel's **Macros** tab (`panel.rs`, `tray::MacroCheck`): the check runs on its own thread on a button
  (`tray::scan_macros`); the name tables are loaded inside `macros::scan` per game version and dropped with it, so the
  tray keeps only the findings. The name download is a worker request (`Request::UpdateNames`, `Work::Names`).
  The findings list is a wide sheet (`W_REPORT`): the window widens while it is open (`panel::fit`), each finding is
  shown in full with wrapped text, there is no hover tip.
- `locales/` — interface strings in five languages (ru, en, de, fr, es), embedded by `i18n.rs`.
- `packaging/` — the Inno Setup installer, the portable marker and the winget manifests.
- `site/` — the landing page (`site/build.py` renders it per language), published by GitHub Pages.
- `.github/workflows/` — CI (fmt, clippy, tests, build, installer), releases, winget submission, Pages.

The module-by-module description is in the root `CLAUDE.md` ("Layout").
