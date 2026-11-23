# 0003 Macro check: panel tab

## Goal
A "Макросы" tab in the tray panel: scan button, summary counts, findings list, copy report, download name data.

## Why
The tray is how the owner uses the tool; the CLI report is not enough for daily use.

## Scope
- Depends on 0001 and 0002.
- New `config::PanelTab::Macros` / `panel::Tab` (shown/hidden/reordered like the others via `Config::shown_tabs`;
  settings chips for it).
- Tab content: scan button (background job like `tray::rescan`, result in `View`), counts per kind, a list button
  opening the findings as a `Sheet` (where: account or character-realm, macro name, line, problem, fix; wheel scroll,
  Esc closes), "Copy report" (text to the clipboard, same path as `diag::report`), "Download name data" button with
  status/spinner (worker `Request`) and the cache state (build, locales).
- `ui-snapshot` views: `macros`, `macros-list`, `macros-busy`, `macros-empty`, `macros-nodata` (all 5 languages and
  `--dark`; check German).
- All strings in 5 locale files; tab height must not exceed the tallest tab behavior (panel height fits tallest tab).
- READMEs (5), CHANGELOG, `docs/images/macros-<lang>[-dark].png`, site screenshots only if the site lists tabs.

## Out of scope
- Editing macros, save-to-file report (clipboard only), automatic scans.

## Acceptance
- Tab scans without blocking the UI; works while a game runs (read-only); downloading is only by button.
- Snapshots look right in ru/en/de; no overflow in German.
- Gates root + `site/` build if images changed.

## Open owner decisions
1. Separate window instead of tab: recommendation tab (reuses Sheets and worker).
2. "Save report" to a file: recommendation skip; clipboard + CLI redirect is enough.

## Decisions (orchestrator, 2026-11-24)
- A panel tab, not a separate window; the report goes to the clipboard only.
- Merged only after the owner checks it on screen.
- The name tables are large (~100 MB in memory per locale while a scan runs): the tray loads them only inside the
  background scan and drops them after; nothing stays resident.
