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

`wow-tools ui-snapshot <out.png> [panel|busy|game|error|confirm|empty|busy-addons|addons|addons-busy|addons-tip|addons-empty|update|update-tip|menu|menu-update|settings|settings-update|settings-backup|settings-exit|settings-addons|settings-every|settings-one|settings-wago] [--dark]` (hidden command; `--lang ru|en|de|fr|es`) renders
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
- `tray.rs`: hidden window, single-instance mutex, runtime-drawn icon whose color is the state, `TaskbarCreated`
  re-add, balloons for manual results and new errors. `view()` builds what the panel/menu show; `act(Action)` runs
  their clicks. A double-click start frees its own console (`GetConsoleProcessList == 1`); after that,
  `Command::spawn` with inherited stdio fails with os error 50, so files are opened with `ShellExecuteW`.
- `gfx.rs`: Direct2D/DirectWrite helpers shared by the UI, modeled on RGBTray (`C:\test\Accs\src\ui.rs`): system
  theme + accent, `R` geometry in DIPs, `text_format`/`text_width`, `Painter`, `Surface`, `create_popup`, `render_png`.
- `panel.rs`: left-click flyout with tabs (remembered while the tray runs): "Бэкап" (status card, backup button, restore
  points with inline confirm) and "Аддоны" (last update, "update now", the last `HISTORY_ROWS` runs from
  `updater::history`, kept in `data\addon-updates.json`, 20 max, written by `app::update_addons`); autostart switch
  under both. The height fits the longer list of the two, so switching tabs does not resize it.
  Pure `draw(view, …)`; closes on deactivate/Esc. `menu.rs`: right-click menu in the same style.
- `settings.rs`: the settings window (a regular titled window; tabs "Общие", "Бэкап", "Аддоны" fixed at the top, the
  cards under them scroll when taller than the screen; the window is sized for the tallest tab): game folder
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
  an addons snapshot labelled `pre-update`; the panel lists those as addons-only restore points. The update runs on
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
- `schedule.rs`: schtasks (`backup`) / HKCU Run value "WoW Tools" (`tray`, or `watch` with `--headless`; the UI
  toggles it), both through `conhost.exe --headless` (no console flash); /TR is limited to 261 chars. Whole days
  (the 1440 default of `schedule interval`) go as `/SC DAILY`: `/SC MINUTE` stops at 1439.
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
  `.bak`/`.old` are excluded (that is why a 155 MB WTF zips to ~13 MB).
- Keybinds and macros may be server-synced (`synchronizeBindings`/`synchronizeMacros`), which can override copied files.
