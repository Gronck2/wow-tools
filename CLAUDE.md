# WoW Tools (crate `wow-tools`)

A Windows tray utility + CLI (Rust) for World of Warcraft. More tools will be added; today it takes zip snapshots
of `WTF` and `Interface/AddOns`, restores them, and copies settings (bindings, macros, chat, CVars, layout, enabled
addons, per-character SavedVariables) from one character to others. Automatic runs: the tray (after every game exit, or
on chosen weekdays at a time — `backup_when`/`backup_days`/`backup_time`), `watch` (same, in a console), a Task Scheduler task (`schedule interval`) or a sign-in autostart
(`schedule logon`, HKCU Run key). The project was first called `wtf_sync`; the folder is still `C:\test\wtf_sync`.

The owner speaks Russian. Talk to them in Russian. Code, comments and commit messages are in English.

## Interface languages

Every user-facing string (windows, balloons, errors, log lines, CLI output, `--help`) lives in
`crates/core/locales/{ru,en,de,fr,es}.toml`, never in the code: `tr!("section.key")` (-> `&'static str`) or
`tr!("section.key", name = value)` (-> `String`, `{name}` in the text). `i18n.rs` embeds the files, falls back to
English, then to the key. The language is `Config::language` (unset = the Windows display language), set by
`i18n::set` at start (`main::early_language` reads `--config` before clap parses) and by the settings picker.
Russian is the source text; write a new string in `ru.toml` and `en.toml`, then add it to `de`/`fr`/`es`.
Paths with backslashes go in single-quoted TOML strings. `--help`: the clap doc comments are English;
`main::localize` replaces them with `cli.<path>.about` / `cli.<path>.<arg>` (`cli.about`, `cli.<arg>` at the top), the
headings and the help/version flags; `chars::Part` implements `ValueEnum` by hand for translated value help.
Tests: every language has every key with the same `{args}`; every `tr!` key exists; no Russian left in non-test
code; every visible command/argument has a help key. Tests assert Russian texts (`i18n::current()` is Russian in tests).
`ui-snapshot ... --lang de` renders a view in a language — check German, it is the longest.

## Build and check

- Toolchain: `stable-x86_64-pc-windows-gnu`. No MSVC on this machine; CI uses MSVC.
- `cargo` is in `%USERPROFILE%\.cargo\bin` and is **not on PATH** in agent shells. Current `windows-*` crates
  need `dlltool`, which comes from WinLibs MinGW (installed via winget, on the user PATH but not in agent shells):
  `export PATH="$HOME/.cargo/bin:$LOCALAPPDATA/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"` (Bash).
- Before committing, run what CI runs: `cargo fmt --all --check`, `cargo clippy --workspace --release --all-targets --locked -- -D warnings`, `cargo test --workspace --release --locked` (the core alone: `cargo test -p wow-tools-core --release --locked`).
  `rustfmt.toml` sets `max_width = 160`.
- Tests live at the bottom of each module and use `tempfile` fake WoW trees. Never write to the real install
  (`C:\Games\World of Warcraft`) from tests or experiments: use `--config <scratch>.toml` with a scratch
  `backup_dir`, and only read-only commands (`status`, `chars`, `list`, `backup`, `copy-char --dry-run`) against it.
- The release exe is replaced only after stopping the running tray (`Stop-Process -Name wow-tools`).
- Edit text with backslashes (`"\u{E713}"` icon glyphs, Windows paths) with the Edit tool, not `sed`: sed eats `\u`
  and turns `\a` into a control character, which broke icons and docs here more than once.

## GitHub, CI and releases

Repo `Gronck2/wow-tools` (public), branch `main`. `.github/workflows/ci.yml` (push to main, PRs, windows-latest,
MSVC): fmt, clippy, tests, build with `--locked`, the installer compiled; exe + setup are run artifacts. `release.yml`:
a `v1.2.3` tag must match `version` in Cargo.toml; it tests, builds and creates a draft release with three forms —
`WoW-Tools-1.2.3.exe` (0.2.x self-update takes the first `.exe` asset, and GitHub lists assets by name, so the
installer is `WoW-Tools-Setup-1.2.3.exe`, sorted after it — never name an exe asset so it sorts first; 0.2.x also
reads the first `SHA-256` line of the notes; newer versions skip names with "setup"), the installer (Inno Setup, `packaging/installer.iss`, per user into
`%LOCALAPPDATA%\Programs\WoW Tools`; its AppId is also `selfupdate::UNINSTALL_KEY`, whose `DisplayVersion` the
self-update keeps current) and `-portable.zip` (`WoW Tools\` with `packaging/portable.txt`) — each with a SHA-256
line and a build provenance attestation (`gh attestation verify`). The notes start with the version's
`## [x.y.z] - date` section of `CHANGELOG.md` (English, user-facing, newest first; release.yml fails without it), so
add to it with every user-visible change. Publish it by hand; `winget.yml` (release
published, or by hand with a tag) fills `packaging/winget/*.yaml` and submits them with wingetcreate as
`Gronck2.WoWTools` (a PR to microsoft/winget-pkgs from the owner's fork) — needs the `WINGET_TOKEN` secret (classic
PAT, public_repo; set 2026-10-31), skipped without it. `winget validate --manifest <dir>` checks filled manifests.
The installer closes the tray with the hidden
`wow-tools quit` (`WM_QUIT_APP`: exits unless busy = exit code 3; an older tray gets WM_CLOSE), restarts it after a
silent (winget) upgrade, runs `schedule logon` for its "Start with Windows" task and `schedule remove` on uninstall. Inno
Setup 6 locally: `%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe`; the .iss is UTF-8 with BOM, its texts in
`[CustomMessages]` (5 languages). Portable mode: `portable.txt` next to the exe (`config::portable_dir`) moves the config to
`<exe dir>\config\config.toml` and `data_dir` to `<exe dir>\data`; `migrate_legacy` is skipped. User docs: `README.md` (English) and `README.{ru,de,fr,es}.md`, kept in step; UI labels in them
quote the locale files; pictures in `docs/images/<view>-<lang>.png` and `<view>-<lang>-dark.png` (`--dark`) come from `ui-snapshot`
(regenerate both after UI changes); the READMEs show them as `<picture>` with a `prefers-color-scheme: dark` source,
so GitHub picks the one matching the reader's theme. Landing page `https://wow-tool.gronck.com`: `python3 site/build.py <out>`
renders `site/template.html` per language from `site/locales/{en,ru,de,fr,es}.toml` (HTML values; English at `/`,
the others at `/<lang>/`; a missing, extra or unused key stops the build; the README links go to that language's
README) and copies the static files, `docs/images/*.png` (`/img`, the page shows the `-dark` ones) and the app icon;
build into the scratchpad and screenshot it with headless Chrome to check. The design system of gold.gronck.com
(tokens in `site/style.css`, self-hosted Open Sans with Latin + Cyrillic); `site.js` fills the download links from
the latest release and opens screenshots large (`a.zoom` -> `<dialog class="lightbox">`). Published by `pages.yml`
(GitHub Pages, custom domain in the repo's Pages settings, Cloudflare DNS: CNAME `wow-tool` -> `gronck2.github.io`,
DNS only). `site/404.html` sends case variants (`/RU/`) and paths under a language to that language. Bump the version in Cargo.toml (and Cargo.lock via a build) before tagging. `.cargo/config.toml`
links the MSVC C runtime statically, so the exe needs no VC++ redistributable. Commit and push only when asked.
License: `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`; every dependency is compatible). `build.rs` writes
`wow-tools.rc` + a VERSIONINFO (ProductName "WoW Tools", version from Cargo.toml) to `OUT_DIR`. No code signing (the owner
will not pay for a certificate): README "Verifying downloads" offers SHA-256 + attestations instead (Microsoft
Store, which signs MSIX for free, was put off as a big rework).

## Seeing the UI without a screen

`wow-tools ui-snapshot <out.png> [panel|panel-drag|busy|game|error|confirm|empty|pinned|unlocked|lock-tip|pin-tip|folder-tip|settings-tip|busy-addons|addons|addons-busy|addons-tip|addons-empty|addons-skipped|update|update-tip|cleanup|cleanup-tip|cleanup-confirm|cleanup-busy|cleanup-done|cleanup-scanning|cleanup-list|profiles|profiles-pick|profiles-apply|profiles-confirm|profiles-busy|profiles-done|profiles-same|profiles-tip|profiles-empty|profiles-search|profiles-game|macros|macro-report|macros-busy|macros-empty|macros-nodata|macros-names|macros-tip|menu|menu-update|settings|settings-update|settings-backup|settings-exit|settings-days|settings-addons|settings-every|settings-one|settings-wago|settings-screenshots|settings-about|tray-icons] [--dark]` (hidden command; `--lang ru|en|de|fr|es`) renders
the panel, the menu or the settings window with demo data, at 1.5x. Read the PNG to check layout after any UI change.
The owner often plays WoW while we work: never screenshot the screen or bring windows to the front to check the UI.

## Layout

A cargo workspace: the root package `wow-tools` (`src/`: exe, CLI, tray, windows, self-update) on the library crate
`wow-tools-core` (`crates/core/`, with `locales/` and `data/`). The core has no window, tray, installer or self-update
and never imports them (`docs/arch/overview.md`: public API, the lock). The program re-exports the core's modules at its
root (`main.rs`), so the window code still says `crate::config`. Below, paths are in `src/` unless a module is in the
core: `i18n`, `config`, `wow`, `snapshot`, `chars`, `lua`, `editmode`, `profiles`, `maintenance`, `app`, `watch`,
`updater`, `curseforge`, `wowup`, `wago`, `elvui`, `macros`, `macro_fix`, `macro_names`, `macro_store`, `http`, `version`,
`errors` and `lock` live in `crates/core/src/`. `Kind` (`snapshot`) and `chars::Part` implement `clap::ValueEnum` behind the core's
optional `clap` feature (on in the program, off for other shells). Core tests: `cargo test -p wow-tools-core`.
- `lock.rs`: one writer per WoW folder. `lock::acquire(&Flavor, what)` takes a fail-fast file lock
  (`File::try_lock`) on `%LOCALAPPDATA%\WoW Tools\locks\<sha256(lowercased flavor folder)[..16]>.lock` (not the portable
  dir: a portable and an installed copy exclude each other; Windows frees it when the process dies); busy = error
  `app.busy` naming the holder (`<pid> <operation>` in `<hash>.info`: a locked file cannot be read by another handle).
  Taken at the outermost entry only (a second handle in one process fails): `app::backup_all` (per flavor; a busy one is
  a report error and the loop goes on), `app::restore_apply`, `app::update_addons` (all selected flavors, for the run),
  `profiles::apply`, `maintenance::clean` (every item) and `check_patches` (snapshot), `main::copy_char` (not dry-run).
  Callees (`safety_snapshot`, `Store::backup`) never take it. Tests use a temp dir for the lock folder.
- `errors.rs` (core): `Coded` = an error with a stable `code` + `params` next to its text (`Display` is the old text, so
  the console and the tray print the same; raised as the error or as a `.with_context(|| Coded::..)`), `classify(&anyhow::Error)`
  -> (code, params) (the first `Coded`, else an `io::Error` -> `io`, else `failed`; never reads text). Raised by the lock
  (`busy`), `ensure_game_closed`/`update_addons` (`game_running`), the lookups (`not_found`), `Config::load`
  (`config_invalid`), `wow::resolve_root` (`no_wow`), `http::get` (`network`). `app::Report` also carries `snapshots`,
  `unchanged` and `failures` without text; `maintenance::clean_item` returns `Cleaned` (`clean` formats it);
  `macros::Finding::owner` (flavor folder, account, realm, character; `serde(skip)`, so `macro-report.json` is unchanged).
- `cli_json.rs`: `--json` (global flag, `main::early_json` reads it before clap; the language is forced to English): one
  `{"v":1,"type":"result"|"error",...}` line on stdout, built from the same core calls as the `cmd_*` text functions,
  never by serializing the core's stored types; `json_unsupported` for `tray`/`watch`/`quit`/`ui-snapshot`/`self-update`/
  `schedule`; `restore` needs `--yes` (stdin is never read); exit 0/1/2. The contract is `docs/cli-json.md` and ADR 0003:
  a changed field needs a changelog line, a breaking one a new `v`. Tests: a fake tree + scratch config, `json!` expectations.
- `http.rs` / `version.rs`: the WinHTTP `get` (user agent from `http::set_user_agent`, called first in `main`) and
  `newer`; `selfupdate` re-exports both.
- `main.rs`: CLI (clap) and printing. No subcommand = `tray`. Runs `config::migrate_legacy` first. Logging goes to
  `%LOCALAPPDATA%\WoW Tools\data\wow-tools.log` (local time) for `--log-to-file` and the tray, to the console otherwise.
- `app.rs`: `Ctx` (config + paths) and the operations shared by CLI, `watch` and tray: `backup_all` (returns a `Report`),
  `restore_plan`/`restore_apply`/`restore_point`, `ensure_game_closed`.
- `watch.rs`: the loop (config re-read every round): `When::GameExit` backs up when a client closes; `When::Schedule`
  at `backup_days` + `backup_time`: `Config::schedule_due(now, last)` is true when the latest scheduled moment (last
  8 days) is after the last snapshot, which also catches up a moment missed while the PC was off; manual and earlier
  snapshots count. `When::EveryDays`: `backup_every_days` after the last snapshot's day, at `backup_time` (caught up the
  same way). If the game held it back or it failed, retried after 10 min or right when the game closes;
  `watch --interval N` is a plain timer instead. `Request` channel (backup now,
  restore) and `Event` callback. Used by `watch` (no requests) and by the tray worker thread.
- `tray.rs`: hidden window, single-instance mutex, tray icon from the exe's resources (`build.rs` +
  `wow-tools.rc` embed `assets\wow-tools.ico`, the gold app icon = a game runs, and `assets\tray-idle.ico`, grey)
  with a dot drawn on for busy (blue) / failed (red) at the lower right and a green up arrow at the upper right for a
  newer WoW Tools (`tray_icon`, the arrow sampled 4x4 from `in_arrow`, drawn under the dot; `ui-snapshot tray-icons`
  shows them), `TaskbarCreated`
  re-add, balloons for manual results and new errors (none with `notifications` off: `update_tray` drops them; the
  new-version balloon is `State::update_balloon`, gated by `notify_updates` instead). `view()` builds what the panel/menu show; `act(Action)` runs
  their clicks. A double-click start frees its own console (`GetConsoleProcessList == 1`); after that,
  `Command::spawn` with inherited stdio fails with os error 50, so files are opened with `ShellExecuteW`.
- `gfx.rs`: Direct2D/DirectWrite helpers shared by the UI, modeled on RGBTray (`C:\test\Accs\src\ui.rs`): system
  theme + accent, `R` geometry in DIPs, `text_format`/`text_width`, `Painter`, `Surface`, `create_popup`, `render_png`.
  Only the menu has `CS_DROPSHADOW`: that shadow is a separate window that ignores z-order, so a pinned panel sent under
  another window left its shadow on top as a ghost; the panel gets DWM's shadow (NC rendering on + a 1 px frame).
- `panel.rs`: left-click flyout with tabs (remembered while the tray runs): "Бэкап" (status card, backup button, restore
  points with inline confirm), "Аддоны" (last update, "update now", the last `HISTORY_ROWS` runs from
  `updater::history`, kept in `data\addon-updates.json`, 20 max, written by `app::update_addons`), "Уборка"
  (`cleanup_lines`: a row per `maintenance::Item` whose button asks "Точно?" in place (`Confirm::Clean`), the biggest
  settings, hover = what a row is about; long lists (orphans, biggest settings) open from a list button as a `Sheet`
  over the tab (wheel scrolls by rows, ✕/Esc closes); `View::scan` comes from `tray::rescan`, a background read started when the
  tab opens and after every cleanup) and "Профили" (a "save" card whose button opens the characters as a `Sheet`
  (`SheetList::SaveFrom`, last played first, `View::chars` read with the cleanup scan; typing goes to `Sheet::search`
  via `WM_CHAR`, Enter picks the first, Esc clears it), saved profiles with "Применить…" (off with a tip while
  `View::running` has the profile's flavor: `Hit::ApplyBlocked`)
  (pick list `ApplyTo`, then "sure?" in the row: `Confirm::ApplyProfile`) and delete; jobs run in the worker
  (`Request::SaveProfile`/`ApplyProfile`), spinner and result in the row) and "Макросы" (`tray::MacroCheck` in
  `View::macros`: "check" starts `tray::scan_macros`, a thread that runs `macros::scan` — the name tables load in it per
  flavor and are dropped, only the findings (`Arc<Scan>`) stay; never started by itself; a counts card per kind and a
  report card with an "open report" button (`Action::OpenMacroReport` -> `report::show`; the panel stays `W` wide, never moves), "download" is `Request::UpdateNames` (`Work::Names`, only on that button; the check on screen is NOT redone: `MacroCheck::names_stale` shows "names updated, check again" until the next check) with the cache state from `macro_names::cached`; the last check is saved to `data\macro-report.json` by `macro_store` (version 1, temp file + rename; a broken or other-version file is ignored with a warn) and loaded in `tray::run`, `MacroCheck::at` is its time); autostart switch under all.
  The height fits the tallest tab, so switching tabs does not resize it (five tabs: `tabs` shrinks the padding to fit). The tabs are `config::PanelTab` (`panel::Tab`), shown as `View::tabs` = `Config::shown_tabs`
  (`panel_tabs` order, missing ones appended, minus `hidden_tabs`, never empty); a tab pressed and moved 6 DIPs is
  dragged (`Drag`, mouse capture, `drag_to` reorders live) and the release saves it (`Action::PanelTabs` ->
  `Config::set_shown_order`, hidden tabs keep their places; `settings::tabs_changed` updates an open settings window).
  Settings → General → "Вкладки панели": one card of chips in `tab_order`, selected = shown (`Row::PanelTabs`,
  `Config::toggle_tab` refuses to hide the last one); the order is changed only by dragging in the panel. The open
  tab, hidden, falls back to the first shown.
  Pure `draw(view, …)`; closes on deactivate/Esc; the header paperclip (`PINNED`, kept while the tray runs) keeps it
  open on deactivate and after the folder/settings buttons (Esc and the tray icon still close it). The panel is not
  topmost (only the menu is); a tray click on a pinned panel that another window overlaps (`panel::covered`: visible,
  not minimized/cloaked/topmost windows above it) brings it to the front instead of closing it.
  The header lock left of the paperclip (`UNLOCKED`, closed at tray start, kept while it runs): open, the header
  outside its buttons answers `WM_NCHITTEST` with `HTCAPTION`; the top left corner goes to `data\panel-position.txt`
  on `WM_EXITSIZEMOVE`, and `open` puts it there whether the lock is open or not (kept on that monitor's work area; at
  the tray when there is no file or the monitor is gone, `gfx::monitor_at`). Every header button has a hover tip
  (`panel.*_tip`; `ui-snapshot lock-tip|pin-tip|folder-tip|settings-tip|unlocked`); the new version button drops its
  number for the icon alone when it would run into the title (`pill_label`). `menu.rs`: right-click menu in the same style.
- `report.rs`: the macro report, a regular resizable window like the settings (`gfx::create_window(.., resizable)`), opened by `Action::OpenMacroReport` (`report::show`: an open one is restored and raised, never a second); `tray` refreshes it with `WM_STATE` (`report::refresh`, `tray::macro_check`) and closes it at exit. A fixed header (`Scan::levels` counters, only levels that occur; "copy" = `Action::CopyMacros`, its result line from `MacroCheck::copied`) over cards scrolled by pixels (wheel, keys, a drawn scroll bar with drag), each finding in full: stripe in the level color (`error` / `warn` / `text2`), "name, line N", where, problem, "-> fix", all wrapped to the width (`layout` measures with `gfx::text_height`, redone on resize and refresh). Size and place go to `data\macro-report-position.txt` ("x y w h", the restored rectangle; used only if its title bar is still on a monitor). `ui-snapshot macro-report` renders it at 900x600 DIPs. Every text of a card is a `Block` (`Role` = font and color): drawn by `gfx::Painter::rich_text` with `Mark`ed ranges bold (problem token in the level color, replaced pieces of a fix in the accent), selectable with the mouse (`Selection` of (block, UTF-16 position), `place_at` via DirectWrite `hit_index`, `selected_text`; Ctrl+C = `Action::CopySelection`, Ctrl+A). The header shows `MacroCheck::checked_at`.
- `settings.rs`: the settings window (a regular titled window; tabs "Общие" (game, panel tabs, system), "Бэкап", "Аддоны", "Скриншоты" (where the cleanup moves them: folder, date folders, folder per flavor), "О программе" (version and self-update switches, config file, diagnostics) fixed at the
  top, the cards under them scroll when taller than the screen; `fit` sizes the window to the open tab, refit on tab
  switches and on changes that add or remove cards, top edge kept): game folder
  (folder picker, "Авто"), flavors as chips, "when" as chips (after game / by weekday / every N days; addons: by weekday /
  periodically / at start) + weekday chips, keep stepper; a selected chip has a check mark (`checked`: a grey accent,
  which `gfx::theme_for` swaps for blue, made selected chips look disabled); `Row::Note` = a text-only card (the
  addons tab's hand-edits warning above the main switch, the screenshots tab's "moved only by hand"),
  switches, backup folder, autostart, a link to
  `config.toml` for the rest (`exclude_ext`). "Диагностика" (`diag::report` -> clipboard): version, game, addon managers
  (`describe`), update settings + history, config and the log tail, the user folder as `%USERPROFILE%`; `live_report`
  (ignored) prints it. Every click saves the config at once. A second start of the exe
  posts `WM_SHOW_SETTINGS` to the running tray instead of exiting silently.
- Windows are shown with `gfx::show` (SetWindowPos), not `ShowWindow`: the first `ShowWindow` of a process started
  hidden (autostart via `conhost --headless`) takes the start mode and the window stays invisible.
- `config.rs`: `%APPDATA%\WoW Tools\config\config.toml`, all fields `#[serde(default)]`; `save` writes a temp file and renames it. `wow-tools config get|set|unset` (`Config::get_key`/`set_key`/`unset_key`: the JSON image of `Config` patched and read back, then `invalid()` for the time formats and ranges; the value argument is JSON else a string, `parse_value`).
- `wow.rs`: install detection (registry, Battle.net, then common paths incl. `X:\Games\World of Warcraft`). A flavor is
  any `_name_` dir (letters, digits: `_dark_realm_2_`) with Battle.net's `.flavor.info` (product code), `WTF`,
  `Interface` or a `Wow*.exe`, so installed-never-started and unknown future flavors count. Warcraft III Reforged
  (and other Blizzard games) also have `_retail_` with a `.flavor.info`: only `wow*` product codes count there and in
  `product.db` (`is_wow_product`). `all_flavors` adds
  flavors Battle.net installed elsewhere, from `%ProgramData%\Battle.net\Agent\product.db` (protobuf; every string of a
  product is collected and the path, `_name_` and version picked out, no schema) — so scratch-config runs still see
  the real installs. `KNOWN` lists folders with titles and product codes (from WowUp and instawow): `_retail_`, `_ptr_`,
  `_xptr_`, `_beta_`, `_classic_` (progression Classic), `_classic_ptr_`, `_classic_beta_`, `_classic_era_` (Era,
  Hardcore, SoD), `_classic_era_ptr_`, `_anniversary_` (TBC Anniversary), `_alpha_`, `_event1_`, ... A Classic slot
  with a 1.60+ client is titled "Forever" (Forever's beta is `_classic_beta_` with `WowB.exe`). Client exe names are
  unreliable, so a running game is a known exe name or any `Wow*.exe` started from a flavor folder (`is_game`);
  `running_in(fl)` blocks only the flavor that runs. All flavors share the WTF/AddOns layout.
- `snapshot.rs`: `<backup_dir>/<flavor>/<wtf|addons>/<YYYYmmdd-HHMMSS>[-label].zip`, paths inside relative to the
  flavor dir. A fingerprint (paths+sizes+mtimes) in `last.fingerprint` skips unchanged trees. Restore extracts to a
  staging dir and swaps folders. `restore` and `copy-char` first take a forced `pre-restore`/`pre-copy` snapshot.
  `snapshot::progress()` is the zip being written (what, bytes done/all); the tray registers `on_progress` (`post_state`
  on every percent), the panel's status line + backup button and the tooltip show it (`panel::busy_line`).
- `updater.rs`: `AddonApp` (CurseForge, WowUp, WowUp-CF, Wago App), detection by exe under
  `%LOCALAPPDATA%\Programs`, `chosen(cfg.update_app)` (one app only: two managers over one AddOns folder fight;
  unset/uninstalled = first installed; the settings show a lone app as a single selected chip), the shared
  `Outcome`, `quiet_command`, `close` (taskkill, then /F), `to_tab_json` (Electron's `JSON.stringify(v, null, "\t")`
  layout) and the last run (`data\addon-update.last`).
- `wago.rs`: the Wago App cannot update addons unattended (only its "Update All" button; no hidden start — from its
  source maps, 2.14.1), but while open it writes WeakAuras updates to `AddOns\WagoAppCompanion` and UI packs to
  `AddOns\WagoUI_Storage`. Chosen as the app, it updates only those (settings then show `wago_weakauras` /
  `wago_ui_packs` switches). It is started with `START_MINIMIZED=1` (window minimized, not hidden), done 15 s after its
  last write to those folders (it writes on load and again when updates arrive; 45 s with no write, 4 min cap), then
  killed after 3 s (a minimized window ignores a polite close). Both switches off = nothing to do (no launch). Its
  addon requests to addons.wago.io are encrypted on purpose (`X-Wago-Iteration` + blob), so addons are not updated
  through it; WowUp has an official Wago provider. The tray tooltip and menu show what is busy (`watch::Work`). It always writes both, so the one switched off is copied to `data\wago-kept` first and put back after (a run
  cut short is undone next time). Game folder from `%APPDATA%\wago-app\config.json`
  (`app-preferences-storage.state.gameInstallation.wowPath`).
- `wowup.rs`: WowUp and WowUp-CF (same source, data in `%APPDATA%\WowUp` / `WowUpCf`). `--hidden --quit` runs one
  auto-update round and quits (it waits for its toast to close when something was updated, so: process exit, or
  "onAutoUpdateInterval" in `logs\main.log` + `addons.json` unchanged for 90 s, 10 min cap). `--hidden` does not cover
the Wago ad consent page (a modal window it opens on every start since 2.2x, shown over everything until a choice
is made), so `updater::hide_windows(pid)` (a `SetWinEventHook` on `EVENT_OBJECT_SHOW`) hides every top-level window
of the process while it runs. Only per-addon
  `autoUpdateEnabled` (off by default) decides, so it is forced on for non-ignored addons in `addons.json` (tab-indented
  electron-store JSON, byte-exact rewrite, `real_file` test) and restored via `data\wowup[-cf]-forced.json`.
  Updated = `installedExternalReleaseId` changed, matched on installationId/providerName/externalId (re-syncs replace
  record ids; an addon it matched nowhere is `providerName: "Unknown"`, no externalId, ignored — keyed by name and
  never counted). Plain WowUp gets Wago downloads with a token from the ad in its window; hidden, it gets none
  ("[wago] no token received", then 401) — checked live 2026-10-12. So `[AddonUpdate] <provider> <id> <name> 'a' -> 'b'`
  log lines that did not end up updated are reported, with that reason; all failed = an error.
- `curseforge.rs`: addon updates through the CurseForge app (its API needs a per-developer key, so we never download
  ourselves). `update()` starts `CurseForge.exe --minimized` (tray, no window; the flag its own login item uses),
  waits until its log (`%APPDATA%\CurseForge\agent\logs\CurseClient\*.json`) has "Completed checking for updates"
  and no install lines for 60 s (10 min cap), then closes it (taskkill, then /F) — only if we started it; an already
  open app is left alone. Open with auto-update off (CurseForge or WowUp) = `updater::NotRun`: an error that is not a run
  (no `last_run`, retried in 30 min like a failure), a history row with `skipped` (yellow "not updated", a repeat of an
  automatic one is not added again), a balloon once per message (`shown_error`). `agent\GameInstances\AddonGameInstance.json` before/after gives the updated addons
  (`installedFile.id` per `addonID`) and the outdated ones (`latestFile.id` differs). `app::update_addons` first takes
  an addons snapshot labelled `pre-update` (not listed in the panel; `restore <id> --kind addons`). The update runs on
  its own schedule (`update_addons`, `update_when`: `schedule` = `update_days`/`update_time`, or `interval` = every
  `update_every` minutes since the last run, or `startup` = a minute after the loop starts unless it updated in the hour
  before; game closed) (`update_skip_in_game`, on by default: also refuses a manual
  run while a client runs, before anything is recorded; the panel button is off then); the last run is in `data\addon-update.last`.
  Spawned tools use null stdio (`quiet_command`), which also avoids os error 50 in the tray.
  With the instance-level `preferenceAutoInstallUpdates` off the app only checks, so for our run it is switched on in
  that JSON (app closed; the file is the source of truth, `agent\database\app.db` has no such setting) and switched
  back after closing. The guids are kept in `data\curseforge-forced.json` until restored (a run cut short is undone
  next time); a copy of the file goes to `AddonGameInstance.json.wow-tools.bak`. serde_json has `preserve_order` +
  `arbitrary_precision` so a rewrite keeps the file byte for byte apart from that flag (the ignored test
  `real_file` checks it against the real file, read-only). Per-addon `preferenceAutoInstallUpdates: false` (manual)
  is respected and reported. Downloading `latestFile.downloadUrl` ourselves was rejected: authors of some addons
  forbid third-party downloads (`allowModDistribution: false`) and the app's records would go stale.
- `chars.rs`: characters at `WTF/Account/<ACC>/<Realm>/<Name>`; names are often Cyrillic, so matching uses `to_lowercase`.
- `macros.rs`: `wow-tools macros`, read-only check of every `macros-cache.txt` (parser, `check`, `scan`, `report`). Tables embedded from `crates/core/data/`: `realms.tsv` (LibRealmInfo, zlib: `THIRD-PARTY.md`; realm folder matched by name, then realm id via `profiles::Roster`, Cyrillic = ruRU "from the name"), `deprecated-api.tsv` (Retail only), `slash-aliases.tsv` (hand table). Findings: `Target::valid` = a Russian realm accepts only ruRU, any other the realm's language or `Target::client` (`client_locale`: `Config.wtf` `textLocale`, else `locale`; + ruRU when the account has a ruRU-realm character); a failing ruRU realm is "bad" (firm in a character's own macro, else "may not work"), another realm "works only with the <lang> client", a realm of unknown language is never bad (`unknown`). `Finding::level` (`Level` Broken/Warn/Info: removed API, too long, brackets, unknown name, firm wrong language and firm alias = Broken; secret values, maybe/client wrong language and soft or maybe alias = Warn; realm-language-unknown ones = Info) and `sort_findings` (level, flavor, scope, macro, line) order `Scan::findings`; `report` marks each level group.

- `macro_fix.rs`: the fixed text of a macro for the report (pure, `build(macro, retail, targets, firm, names)` -> `MacroFix { variants }`; `Variant` = lang, text, chars, marks (byte ranges of what was replaced, highlighted by the report), check (a table function was replaced: "check in game" note), over (> 255 = no text, only the mark), rest (findings the text leaves, TooLong excluded)). Language per target (`target_lang`): ru realm = ruRU; own macro = realm language, account macro = `Target::client_main` (else realm). Targets of one language form a group; texts that come out equal merge; a language with no cached tables gives no variant. Per line (`fix_line`): `/run` `/script` -> `rewrite_code` (column 5 of `deprecated-api.tsv`: `drop-in` as is; `first:<field>:<min>-<max>` = table functions -> `(C_X.F(args) or {}).field` only when `single_value(before, after)` says one value is used: operand of an operator, condition, one-target assignment, parenthesised, non-last list element, sole argument of `tostring` (not `print`: it shows every return), and the argument count fits; `one-arg` = `GetSpellTexture`/`IsUsableSpell` with one argument; a call outside strings/comments, not a field or definition); lines with unbalanced `[ ]` stay; names via `Names::retarget` (needs `valid` of some target wrong, one unambiguous `translate_exact`, spans from `macro_names::name_spans`); an alias of a language the group lacks -> `canonical_command`. `Finding::fixed` is the same `Arc` on every finding of a macro; `macros::first_fixes` marks the first (the only one shown: `report.rs` block + Copy button -> `Action::CopyFix(Arc<str>)`, `macros::report` via `macro_fix::render`). Rows verified against warcraft.wiki.gg and Blizzard's API documentation (wow-ui-source `live`): `UseContainerItem` (3rd/4th arguments changed) is not rewritten. Names with an inline icon (`|A:..|a`) are skipped by the checks and counted in `rest` (`macro_names::marked`).
- `macro_names.rs`: name data for `macros`: `--update-names` (only on request) downloads wago.tools `SpellName` / `ItemSparse` CSVs (item names = its `Display_lang` column; `ItemSearchName` lacks many consumables; ~50-60 MB per locale, so `selfupdate::http::get` caps at 128 MB; a cache without the `item-sparse` entry is not current) per product (`wow::product`) and realm locale (+ the client locales of `Target::valid`, + enUS) into `data\names\<product>-<locale>.zip` (+ `.meta` = `<build of /api/builds/<product>/latest>\titem-sparse`; current = no download; a meta without the tab part is stale); a table over 98% identical to enUS is rejected. `check` finds names (`/cast /use /castsequence /castrandom /userandom #showtooltip #show` + aliases) in the wrong language (with the realm-language name) or unknown; no cache = `Scan::missing` hint. `selfupdate::http::get` caps a body at 128 MB.
- `profiles.rs`: UI profiles, zips in `<backup_dir>\profiles\<flavor>\` (`profile.json` meta, `addons.json`, `character/`
  files, `account/edit-mode-cache-account.txt`): every file of the character folder + its `SavedVariables\*.lua`, except
  `SKIP` (`cache.md5`; bindings, click bindings and macros belong to the character: neither saved nor applied, older
  profiles have them), and for AceDB addons (account
  `SavedVariables` with `["profileKeys"]`) the profile the character uses + its table (and the namespaces'). `apply`
  (flavor closed, `pre-profile` WTF snapshot) copies the files and sets `profileKeys["Name - Realm"]` to that profile:
  shared on the same account; elsewhere added, as "<name> (<source key>)" when a different one of that name exists.
  Edit Mode (`editmode.rs`, `put_layouts`): `edit-mode-cache-character.txt` = 6 specialization slots (Lua index into
  presets 1/2 + account layouts from 3 + character layouts; 0 = none chosen, -1 = no such spec), then the character's
  layouts (`<byte len> <name> <n>` + n frames of 10 tokens); the account file = version, count, settings, layouts.
  `merge` adds the layouts the source uses to the target's own (an identical one on the target's account is used; a
  taken name gets " (<source name>)"; 5 per character, `full` beyond), slots the source left unset get its most used
  one. Unknown format = left alone. `live_files`/`live_merge` (ignored) check the real files read-only.
  `characters` (the panel's pick lists) leaves out folders of characters that are gone (`Roster`): the account's
  `character-list-order.txt` has the current GUIDs (`<realm id>-<hex>`), names come from addon SavedVariables keyed by
  `["Player-<guid>"]` with a `name` (and `realm`, else from the realm id or the only matching folder). Hidden only
  when sure: the realm's GUIDs are all named, or the realm is not on the list while every listed realm is known.
  `CharEntry::look` (class + faction; the game keeps neither in WTF) comes from the same GUID records (`class`/
  `classKey`, `faction`, or the side of `race`/`raceKey`; a GUID is read on until all three are known, ~0.3 s for
  150 characters) and then ElvUI's `ElvDB.class`/`.faction[realm][name]`; the panel draws a faction badge before the
  name and the name in the class color (`panel::class_color`, dimmed on light), saved profiles by their character.
  `lua.rs` reads and edits SavedVariables text in place (keeps the rest byte for byte). `wow-tools profile`.
- `schedule.rs`: schtasks (`backup`) / HKCU Run value "WoW Tools" (`tray`, or `watch` with `--headless`; the UI
  toggles it), both through `conhost.exe --headless` (no console flash); /TR is limited to 261 chars. Whole days
  (the 1440 default of `schedule interval`) go as `/SC DAILY`: `/SC MINUTE` stops at 1439.
- `maintenance.rs`: game patches and cleanup. `check_patches` (watch loop, once a minute) compares each selected
  flavor's client version (`wow::all_flavors`, from Battle.net's product.db) with `data\game-versions.json`; a new
  version while that client is closed = patched, not started yet, so WTF + AddOns are snapshotted forced with label
  `pre-patch` (`Event::Patch` -> balloon). First sight of a flavor is only recorded. `outdated(fl)` reads every
  `<Addon>*.toc` of an addon folder, all `## Interface*` numbers; judged by major.minor (patch digit ignored): on
  Retail (>= 80000) against every Retail-era number, on Classic against the same major; no number for this game =
  not judged; `Blizzard_*` skipped. Shown in full in Settings -> Addons ("Совместимость", `outdated_blocks`: the card
  grows and turns `warn`-colored) and `wow-tools outdated`.
  `scan(flavors)` (read-only, ~0.4 s on the real 216-character WTF) / `clean(ctx, Item)`: `Orphans` = SavedVariables
  (`.lua`, `.lua.bak`) under any `SavedVariables` folder whose addon folder is not in `Interface\AddOns` (no AddOns
  folder = not judged; `Blizzard*` kept); `Backups` = other `.bak`/`.old` in WTF; `Logs` = `Logs\WoWCombatLog*.txt`
  older than 7 days; `Screenshots` = moved to `screenshots_dir(cfg)` (default `Pictures\World of Warcraft`),
  `<flavor title>\` (or the flavor's own folder from `screenshots_dirs`, `flavor_screenshots_dir`) if
  `screenshots_per_flavor`, a date folder per `ShotsBy` (date from `WoWScrnShot_MMDDYY_HHMMSS`,
  else mtime; never overwrites, copy+remove across drives); `Cache` = `Cache` + `Errors`. Orphans take a `pre-cleanup` WTF
  snapshot first (`.bak`/`.old` are not in snapshots, so Backups need none); only the cache of a running flavor is skipped (`Item::needs_closed_game`: orphans belong to
  addons the game has not loaded, `.bak` copies are made again on exit, logs and screenshots are never read back).
  The panel shows a spinner in place of the button while it runs and the result (`tray` `cleaned`) in the row until
  it closes (balloons are hidden while a fullscreen game runs). Shown in the panel's "Уборка" tab (cleanup is actions, not
  settings). `wow-tools clean [items]`.
- `elvui.rs`: ElvUI is on no addon manager, so `app::update_addons` also updates it itself (`update_elvui`, on by
  default; works with no manager installed). `api.tukui.org/v1/addon/elvui` gives `version` + `url` of one zip for all
  clients (`ElvUI`, `ElvUI_Libraries`, `ElvUI_Options`). Per selected flavor that has `AddOns\ElvUI`: newer (by
  `selfupdate::newer`) or mixed `## Version:` over its `.toc` files (an older version's leftover `ElvUI_Mainline.toc`
  wins over `ElvUI.toc` in game) = reinstall. Extracted to `Interface\.wow-tools-elvui\new`, the folders swapped whole
  (old ones to `old\`, rolled back on failure, `recover` next time); a zip with anything not `ElvUI*` is refused;
  plugins (`ElvUI_WindTools`) untouched. Its own history row (app "ElvUI", "from → to") when it updated or failed.
  `live_tukui` (ignored) runs the real site.
- `selfupdate.rs`: updates of WoW Tools itself from the GitHub releases of `CARGO_PKG_REPOSITORY` (network access of
  the app: this and tukui.org for ElvUI; README "Privacy" says so). WinHTTP (system proxy and certificates, no TLS
  crate; `selfupdate::http::get` is shared with `elvui`); GET
  `api.github.com/repos/<repo>/releases/latest` (published releases only; 404 = none or a private repo, not an
  error). The exe asset is checked against the asset's GitHub `digest` (or the `SHA-256: ...` line release.yml writes
  into the notes) and its size; no hash = no install. `install` renames the running exe to `<exe>.old` and copies the
  new one in (rolled back if the copy fails); `exe_path()` caches `current_exe()` from before the rename. The tray
  (`tray::self_update_loop`, its own thread: a minute after start, then hourly if `check_updates` and 24 h passed;
  `data\self-update.json` keeps the last check, the version a balloon was shown for, and the version being installed
  for a "updated" balloon in the new exe) shows it as a header button in the panel, the version row of the menu,
  the tray icon's arrow and tooltip, and rows in Settings → General. The balloon (`notify_update`) comes once per
  version; found while a game runs it waits for `Event::Game(None)` (`update_waiting`; `was_notified` at start
  catches one a closed tray never told). After `install` it posts `WM_RESTART`: waits while a job runs, then starts
  `conhost --headless <exe> tray --wait-pid <pid>` (hidden arg: the new tray waits for the old one to exit, so the
  single-instance mutex is free) and exits. `self-update [--check]` does the same from the console (no restart).
  `live_github` (ignored) runs the real API and a real download against a public repo.

## Gotchas

- WoW writes WTF and SavedVariables on logout/exit, so snapshots of the running flavor are skipped and `restore`/`copy-char`
  of it refuse to run (other flavors go on). The panel lists the newest points of all flavors, each flavor at least once.
  Safety snapshots (`app::safety_snapshot`: `pre-cleanup`, `pre-profile`, `pre-copy`): `pre-cleanup` and `pre-profile` are listed; one
  taken in the last 10 minutes that is still the newest covers the next action (no new one) while WTF is as the last
  action left it (`app::safety_done` writes `safety.fingerprint` in the series folder). `.bak` cleanup snapshots only
  when `exclude_ext` keeps `.bak`/`.old` in snapshots. The panel's confirmations and pick lists hold positions;
  `panel::follow` moves them to the same point/profile/character when the lists change (or drops them).
  `.bak`/`.old` are excluded (that is why a 155 MB WTF zips to ~13 MB).
- Keybinds and macros may be server-synced (`synchronizeBindings`/`synchronizeMacros`), which can override copied files.

## Agent pipeline

Non-trivial work goes through the `dev-pipeline` plugin (`/dev-pipeline:run-task`); project specifics are the Pipeline
contract. Do not add same-named `.claude/agents/` copies unless a deliberate override is needed.

## Pipeline contract

- **Base branch:** main
- **PR target:** main
- **Forge:** gh
- **Branch naming:** task/NNNN-slug; docs-only work on docs/<slug>
- **Docs layout:** tasks docs/tasks/, plans docs/plan/ (shared NNNN numbering), ADRs docs/adr/ (immutable), living
  docs: docs/arch/overview.md. docs/images/ stays as is (screenshots from `ui-snapshot`). Docs language: English.
- **Test paths:** inline `#[cfg(test)]` modules at the bottom of src/*.rs and crates/core/src/*.rs (no tests/ dir); site/ has no tests
  (build.py validates)
- **Gate commands** (per part; run a part's commands when the diff touches files under it, from the repo root). The
  shell needs the PATH from "Build and check" (cargo + WinLibs `dlltool`):
  - root (Rust workspace; src/, crates/, build.rs, Cargo.*, assets/, packaging/):
    - format: `cargo fmt --all --check`
    - lint: `cargo clippy --workspace --release --all-targets --locked -- -D warnings`
    - tests: `cargo test --workspace --release --locked`
  - `site/` (also when docs/images/ or assets/wow-tools.ico change):
    - build: `uv run --no-project python site/build.py <scratchpad>/site` (`python3` here is only the Store alias)
  - No `cargo build --release --locked` gate: it fails while the tray runs from target\release, and clippy and the
    tests already build release. Never stop the tray to make a build pass.
- **Security-shaped paths:** src/selfupdate.rs, crates/core/src/lock.rs, crates/core/src/http.rs, crates/core/src/elvui.rs, src/schedule.rs, crates/core/src/config.rs, src/main.rs,
  crates/core/src/snapshot.rs, crates/core/src/curseforge.rs, crates/core/src/wowup.rs, crates/core/src/wago.rs, build.rs, wow-tools.rc, .cargo/config.toml,
  packaging/installer.iss, packaging/winget/, .github/workflows/release.yml, .github/workflows/winget.yml,
  .github/workflows/pages.yml, site/site.js
- **Merge/release policy:** PR opened without asking; only the owner merges (squash), only with green CI; direct
  pushes to main, version bumps, tags (`v1.2.3` = Cargo.toml version), publishing the draft release and force-push
  only on the owner's explicit request; never propose merging a PR whose CI is red or pending. Every user-visible
  change adds to the `## [x.y.z]` section of CHANGELOG.md (see "GitHub, CI and releases").
- **Extra rules:** see "Interface languages" (every user-facing string in crates/core/locales/{ru,en,de,fr,es}.toml via `tr!`,
  Russian is the source; site texts in site/locales/*.toml); "Build and check" (never touch the real WoW install from
  tests or experiments, `--config <scratch>.toml`; Edit tool for text with backslashes); "Seeing the UI without a
  screen" (UI is checked only via `wow-tools ui-snapshot`; never screenshot the screen or bring windows to the
  front). Never stop, restart or replace the running tray or the game. READMEs in all languages kept in step.
  Commit and push only when asked.
