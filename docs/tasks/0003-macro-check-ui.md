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

## Revision 1 (owner feedback)
The findings list (rows cut to two lines, full text in a hover tip) was unreadable. Decisions:
- The report is shown in the window itself: no hover tip for findings, every text wrapped in full, never cut.
- While the report is open the panel is wider (`W_REPORT` = 640 DIPs); every other tab, and the Macros tab with
  no report open, stays 360 DIPs and renders byte for byte as before. The header buttons and the divider/footer
  follow the current width; the tab strip keeps its 360 layout, left aligned.
- A finding shows: macro and line, where it is (account or character-realm), what is wrong and the fix, each on its
  own wrapped line(s); rows have the height of their text, the list scrolls by whole findings (wheel, scroll bar).
- The window grows keeping the taskbar side; if it would leave the screen it grows to the left and returns on
  close. `data\panel-position.txt` keeps the narrow window's corner.
- `ui-snapshot macros-list-tip` is removed; `docs/images/macros-<lang>[-dark].png` now come from `macros-list`.

## Revision 2 (owner feedback)
Revision 1 (the widening panel, commits 2d0201f / 48d5075 / 5026a22) is superseded: the owner rejected it. Final
decisions:
- The panel never widens or moves: always `W` = 360 DIPs. `W_REPORT`, the width-changing `fit`, `Panel::shift` and the
  wide macro sheet (`SheetList::Macros`, its rows and scroll code) are gone; `panel.rs` is back to 7c2e2d9 plus the
  new button. The Macros tab keeps "Check macros", the status card, the counts card and the names download card; the
  list button became a report card with an "Open report" button (only with findings); Copy moved to the report window.
- The report is `src/report.rs`: a regular titled window ("WoW Tools: macro problems"), resizable, minimize/maximize,
  900 x 600 DIPs at first on the cursor's monitor, then the remembered `data\macro-report-position.txt` ("x y w h",
  used only while its title bar is on a monitor). Opening it again restores and raises the open window. A fixed
  header (found / macros checked, counters per level that occurs, Copy and its result) and cards scrolled by pixels
  (wheel, keys, draggable bar); each card shows macro and line, where (game version only when several), the problem
  and the fix, wrapped in full; relayout on resize. A finished re-check refreshes it through `WM_STATE`.
- Severity (`macros::Level`, `Finding::level`), worst first:
  - Broken (red): `Deprecated`, `TooLong`, `Brackets`, `UnknownName`; `WrongLanguage` with `How::Firm`; `Alias` with
    `bad` non-empty and `firm`.
  - Warn (yellow): `Secret` (the message says values "may be hidden in combat", so not a certain failure);
    `WrongLanguage` with `How::Maybe` or `How::Client`; `Alias` with `bad` non-empty and not firm, or `soft`
    non-empty.
  - Info (grey): `WrongLanguage` with `How::Realm`; `Alias` with only `unknown` realms.
- `macros::scan` sorts findings by level, flavor, scope, macro name, line, so the panel report, the clipboard and
  `wow-tools macros` agree; the text report starts each level/flavor/scope group with `[level] flavor / scope`.
- `ui-snapshot macro-report` (900 x 600 DIPs at 1.5x, `--dark`, 5 languages); `macros-list` is removed.
  `docs/images/macros-<lang>[-dark].png` are the Macros tab again, `macro-report-<lang>[-dark].png` are new.
