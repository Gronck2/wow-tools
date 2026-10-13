# WoW Tools (crate `wow-tools`)

A Windows tray utility + CLI (Rust) for World of Warcraft. More tools will be added; today it takes zip snapshots
of `WTF` and `Interface/AddOns`, restores them, and copies settings (bindings, macros, chat, CVars, layout, enabled
addons, per-character SavedVariables) from one character to others. Automatic runs: the tray (after every game exit, or
on chosen weekdays at a time — `backup_when`/`backup_days`/`backup_time`), `watch` (same, in a console), a Task Scheduler task (`schedule interval`) or a sign-in autostart
(`schedule logon`, HKCU Run key). The project was first called `wtf_sync`; the folder is still `C:\test\wtf_sync`.

The owner speaks Russian. Talk to them in Russian. Code, comments and commit messages are in English.

## Interface languages

Every user-facing string (windows, balloons, errors, log lines, CLI output, `--help`) lives in
`locales/{ru,en,de,fr,es}.toml`, never in the code: `tr!("section.key")` (-> `&'static str`) or
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
- Before committing, run what CI runs: `cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings`, `cargo test --release`.
  `rustfmt.toml` sets `max_width = 160`.
- Tests live at the bottom of each module and use `tempfile` fake WoW trees. Never write to the real install
  (`C:\Games\World of Warcraft`) from tests or experiments: use `--config <scratch>.toml` with a scratch
  `backup_dir`, and only read-only commands (`status`, `chars`, `list`, `backup`, `copy-char --dry-run`) against it.
- The release exe is replaced only after stopping the running tray (`Stop-Process -Name wow-tools`).
- Edit text with backslashes (`"\u{E713}"` icon glyphs, Windows paths) with the Edit tool, not `sed`: sed eats `\u`
  and turns `\a` into a control character, which broke icons and docs here more than once.

## GitHub, CI and releases

Repo `Gronck2/wow-tools` (private), branch `main`. `.github/workflows/ci.yml` (push to main, PRs, windows-latest,
MSVC): fmt, clippy, tests, build with `--locked`; the exe is a run artifact. `release.yml`: a `v1.2.3` tag must match
`version` in Cargo.toml; it tests, builds and creates a draft release with `WoW-Tools-1.2.3.exe` and its SHA-256 —
publish it by hand. User docs: `README.md` (English) and `README.{ru,de,fr,es}.md`, kept in step; UI labels in them
quote the locale files; pictures in `docs/images/<view>-<lang>.png` come from `ui-snapshot` (regenerate after UI
changes). Bump the version in Cargo.toml (and Cargo.lock via a build) before tagging. `.cargo/config.toml`
links the MSVC C runtime statically, so the exe needs no VC++ redistributable. Commit and push only when asked.

## Seeing the UI without a screen

`wow-tools ui-snapshot <out.png> [panel|busy|game|error|confirm|empty|busy-addons|addons|addons-busy|addons-tip|addons-empty|update|update-tip|cleanup|cleanup-tip|cleanup-confirm|cleanup-busy|cleanup-done|cleanup-scanning|cleanup-list|profiles|profiles-pick|profiles-apply|profiles-confirm|profiles-busy|profiles-done|profiles-same|profiles-tip|profiles-empty|profiles-search|profiles-game|menu|menu-update|settings|settings-update|settings-backup|settings-exit|settings-addons|settings-every|settings-one|settings-wago|settings-screenshots|tray-icons] [--dark]` (hidden command; `--lang ru|en|de|fr|es`) renders
the panel, the menu or the settings window with demo data, at 1.5x. Read the PNG to check layout after any UI change.
The owner often plays WoW while we work: never screenshot the screen or bring windows to the front to check the UI.

## Layout

- `main.rs`: CLI (clap) and printing. No subcommand = `tray`. Runs `config::migrate_legacy` first. Logging goes to
  `%LOCALAPPDATA%\WoW Tools\data\wow-tools.log` (local time) for `--log-to-file` and the tray, to the console otherwise.
- `app.rs`: `Ctx` (config + paths) and the operations shared by CLI, `watch` and tray: `backup_all` (returns a `Report`),
  `restore_plan`/`restore_apply`/`restore_point`, `ensure_game_closed`.
- `watch.rs`: the loop (config re-read every round): `When::GameExit` backs up when a client closes; `When::Schedule`
  at `backup_days` + `backup_time`: `Config::schedule_due(now, last)` is true when the latest scheduled moment (last
  8 days) is after the last snapshot, which also catches up a moment missed while the PC was off; manual and earlier
  snapshots count. If the game held it back or it failed, retried after 10 min or right when the game closes;
  `watch --interval N` is a plain timer instead. `Request` channel (backup now,
  restore) and `Event` callback. Used by `watch` (no requests) and by the tray worker thread.
- `tray.rs`: hidden window, single-instance mutex, tray icon from the exe's resources (`build.rs` +
  `wow-tools.rc` embed `assets\wow-tools.ico`, the gold app icon = a game runs, and `assets\tray-idle.ico`, grey)
  with a dot drawn on for busy (blue) / failed (red) (`tray_icon`; `ui-snapshot tray-icons` shows them), `TaskbarCreated`
  re-add, balloons for manual results and new errors. `view()` builds what the panel/menu show; `act(Action)` runs
  their clicks. A double-click start frees its own console (`GetConsoleProcessList == 1`); after that,
  `Command::spawn` with inherited stdio fails with os error 50, so files are opened with `ShellExecuteW`.
- `gfx.rs`: Direct2D/DirectWrite helpers shared by the UI, modeled on RGBTray (`C:\test\Accs\src\ui.rs`): system
  theme + accent, `R` geometry in DIPs, `text_format`/`text_width`, `Painter`, `Surface`, `create_popup`, `render_png`.
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
  (`Request::SaveProfile`/`ApplyProfile`), spinner and result in the row); autostart switch under all. The height fits the tallest tab, so switching tabs
  does not resize it.
  Pure `draw(view, …)`; closes on deactivate/Esc. `menu.rs`: right-click menu in the same style.
- `settings.rs`: the settings window (a regular titled window; tabs "Общие" (game + system only), "Бэкап", "Аддоны", "Скриншоты" (where the cleanup moves them: folder, date folders, folder per flavor) fixed at the
  top, the cards under them scroll when taller than the screen; `fit` sizes the window to the open tab, refit on tab
  switches and on changes that add or remove cards, top edge kept): game folder
  (folder picker, "Авто"), flavors as chips, "when" (after game / by schedule) + weekday chips, keep stepper,
  switches, backup folder, autostart, a link to
  `config.toml` for the rest (`exclude_ext`). Every click saves the config at once. A second start of the exe
  posts `WM_SHOW_SETTINGS` to the running tray instead of exiting silently.
- Windows are shown with `gfx::show` (SetWindowPos), not `ShowWindow`: the first `ShowWindow` of a process started
  hidden (autostart via `conhost --headless`) takes the start mode and the window stays invisible.
- `config.rs`: `%APPDATA%\WoW Tools\config\config.toml`, all fields `#[serde(default)]`.
- `wow.rs`: install detection (registry, Battle.net, then common paths incl. `X:\Games\World of Warcraft`). A flavor is
  any `_name_` dir (letters, digits: `_dark_realm_2_`) with Battle.net's `.flavor.info` (product code), `WTF`,
  `Interface` or a `Wow*.exe`, so installed-never-started and unknown future flavors count. `all_flavors` adds
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
  "onAutoUpdateInterval" in `logs\main.log` + `addons.json` unchanged for 90 s, 10 min cap). Only per-addon
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
  open app is left alone. `agent\GameInstances\AddonGameInstance.json` before/after gives the updated addons
  (`installedFile.id` per `addonID`) and the outdated ones (`latestFile.id` differs). `app::update_addons` first takes
  an addons snapshot labelled `pre-update` (not listed in the panel; `restore <id> --kind addons`). The update runs on
  its own schedule (`update_addons`, `update_when`: `schedule` = `update_days`/`update_time`, or `interval` = every
  `update_every` minutes since the last run; game closed) (`update_skip_in_game`, on by default: also refuses a manual
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
- `profiles.rs`: UI profiles, zips in `<backup_dir>\profiles\<flavor>\` (`profile.json` meta, `addons.json`, `character/`
  files): every file of the character folder (no `cache.md5`) + its `SavedVariables\*.lua`, and for AceDB addons (account
  `SavedVariables` with `["profileKeys"]`) the profile the character uses + its table (and the namespaces'). `apply`
  (flavor closed, `pre-profile` WTF snapshot) copies the files and sets `profileKeys["Name - Realm"]` to that profile:
  shared on the same account; elsewhere added, as "<name> (<source key>)" when a different one of that name exists.
  `edit-mode-cache-character.txt` starts with the Edit Mode layout per specialization slot (6 numbers, 0/1 = presets);
  slots the source never used get its layout (`fill_layout_slots`), else other specs fall back to the default.
  `characters` (the panel's pick lists) leaves out folders of characters that are gone (`Roster`): the account's
  `character-list-order.txt` has the current GUIDs (`<realm id>-<hex>`), names come from addon SavedVariables keyed by
  `["Player-<guid>"]` with a `name` (and `realm`, else from the realm id or the only matching folder). Hidden only
  when sure: the realm's GUIDs are all named, or the realm is not on the list while every listed realm is known.
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
  `<flavor title>\` if `screenshots_per_flavor`, a date folder per `ShotsBy` (date from `WoWScrnShot_MMDDYY_HHMMSS`,
  else mtime; never overwrites, copy+remove across drives); `Cache` = `Cache` + `Errors`. Orphans take a `pre-cleanup` WTF
  snapshot first (`.bak`/`.old` are not in snapshots, so Backups need none); only the cache of a running flavor is skipped (`Item::needs_closed_game`: orphans belong to
  addons the game has not loaded, `.bak` copies are made again on exit, logs and screenshots are never read back).
  The panel shows a spinner in place of the button while it runs and the result (`tray` `cleaned`) in the row until
  it closes (balloons are hidden while a fullscreen game runs). Shown in the panel's "Уборка" tab (cleanup is actions, not
  settings). `wow-tools clean [items]`.
- `selfupdate.rs`: updates of WoW Tools itself from the GitHub releases of `CARGO_PKG_REPOSITORY` (the only network
  access of the app; README "Privacy" says so). WinHTTP (system proxy and certificates, no TLS crate); GET
  `api.github.com/repos/<repo>/releases/latest` (published releases only; 404 = none or a private repo, not an
  error). The exe asset is checked against the asset's GitHub `digest` (or the `SHA-256: ...` line release.yml writes
  into the notes) and its size; no hash = no install. `install` renames the running exe to `<exe>.old` and copies the
  new one in (rolled back if the copy fails); `exe_path()` caches `current_exe()` from before the rename. The tray
  (`tray::self_update_loop`, its own thread: a minute after start, then hourly if `check_updates` and 24 h passed;
  `data\self-update.json` keeps the last check, the version a balloon was shown for, and the version being installed
  for a "updated" balloon in the new exe) shows it as a header button in the panel, the version row of the menu and
  two rows in Settings → General. After `install` it posts `WM_RESTART`: waits while a job runs, then starts
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
