# Architecture overview

WoW Tools (crate `wow-tools`) is a Windows tray utility and CLI in Rust for World of Warcraft: zip snapshots of `WTF`
and `Interface/AddOns` with restore, copying settings between characters, UI profiles, addon updates through the
installed addon managers (CurseForge, WowUp, Wago App) plus ElvUI, cleanup, and self-update from GitHub releases.

- `src/` — one binary. `main.rs` (CLI, clap) and `tray.rs` (tray, default with no subcommand) share the operations in
  `app.rs`; `watch.rs` runs the automatic loop; UI windows (`panel.rs`, `menu.rs`, `settings.rs`) draw with
  Direct2D/DirectWrite through `gfx.rs`. Tests are inline `#[cfg(test)]` modules.
- `macros.rs` — the macro check (`wow-tools macros`): parser of `macros-cache.txt`, offline checks; tables in `data/*.tsv`.
- `locales/` — interface strings in five languages (ru, en, de, fr, es), embedded by `i18n.rs`.
- `packaging/` — the Inno Setup installer, the portable marker and the winget manifests.
- `site/` — the landing page (`site/build.py` renders it per language), published by GitHub Pages.
- `.github/workflows/` — CI (fmt, clippy, tests, build, installer), releases, winget submission, Pages.

The module-by-module description is in the root `CLAUDE.md` ("Layout").
