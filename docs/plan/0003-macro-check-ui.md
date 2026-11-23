# Plan 0003 Macro check UI

## Approach
Follow the "Уборка" tab: a background scan stored in `View`, a `Sheet` for the long list, buttons dispatching
`Request`s to the existing worker.

## Steps
1. `config.rs`: add `PanelTab::Macros` (serde name, `shown_tabs` default order appended last, so existing configs
   work); `settings.rs` chip row picks it up.
2. `tray.rs`: `View::macros` (last report, running flag, name-data state); `rescan`-like `scan_macros()` thread;
   `Request::UpdateNames`; `Action::ScanMacros`, `Action::CopyMacros`.
3. `panel.rs`: `Tab::Macros` drawing (status card with counts, buttons), `SheetList::Macros` rows from `Finding`s,
   hit-testing and tips like the other tabs; confirm nothing (read-only).
4. `app.rs`/`macros.rs`: expose `report` text for copy.
5. `ui-snapshot` demo data and views; render ru/en/de, light and dark; read PNGs to verify layout.
6. Locale keys x5, README x5, CHANGELOG, images.

## Files likely touched
`src/config.rs`, `src/panel.rs`, `src/tray.rs`, `src/settings.rs`, `src/main.rs` (snapshot names), `src/macros.rs`,
`locales/*.toml`, `README*.md`, `CHANGELOG.md`, `docs/images/`, `docs/arch/overview.md`.

## Tradeoffs
- Tab vs window: tab reuses all the panel machinery; a window would duplicate it.
- Clipboard report only: no file dialogs.
- Never stop the running tray to test; check only through `ui-snapshot`.
