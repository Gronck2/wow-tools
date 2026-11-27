# Plan 0010

One PR, commits per step; text output and the tray never change. Independent of 0009; both edit `update_data` in `src/cli_json.rs`
(0010 enriches it, 0009 only calls it). If both are open, land 0010 first: the `addons` event then carries the lists with no extra work.

## Facts read
- `app::update_addons` (`crates/core/src/app.rs`): `app.update(..).map(|o| o.summary())` drops the `Outcome`; `record(title, &Result<String>)`
  writes `updater::Entry{time, app, ok, text, manual, skipped}`; ElvUI gets its own row from `elvui::update(&flavors) -> Result<Option<String>>`.
  A repeat `NotRun` row of an automatic run is not written again.
- `updater::Outcome{updated: Vec<String>, outdated: Vec<String>, note}`; `summary()` makes the localized text; `NotRun` is a typed error.
- `curseforge.rs`: `Instance{path, name, auto_update, addons: HashMap<addonID,(name, fileId)>, outdated, manual}`; `updated(before, after)`
  returns sorted names. `parse_instances` reads `installedFile.id`, `latestFile.id`, per-addon `preferenceAutoInstallUpdates`.
- `wowup.rs`: `Addon{key, name, installation, installed (release id else version), latest, ..}`; `updated(before, after)` keys by `key`;
  `attempted(log)` parses `[AddonUpdate] provider id Name 'a' -> 'b'` but returns names only; `failed` = attempted minus updated; the
  all-failed case `bail!`s; `NO_TOKEN_MARK`; `installations(text)` gives (id, location).
- `wago.rs::update`: `updated` holds `tr!("wago.apply_in_game", what = label(what))` (a sentence) for each folder whose state changed.
- `elvui.rs::update`: per flavor `current = versions[0]`, `mixed`, `latest.version`; returns the history text only.
- `cli_json::update_data` builds `entries[]` without text; `docs/cli-json.md` says lists are not available yet.

## Steps
1. **Types in `updater.rs`** (serde, `Clone`, `PartialEq`, `Debug`): `ChangeKind {Addon, Weakauras, UiPacks}` (`snake_case`; `UiPacks` -> `ui_packs`),
   `Change{name, kind, from: Option<String>, to: Option<String>, flavor: Option<String>}`, `Failed{name, from, to, flavor, reason: FailReason{NoToken, Unknown}}`,
   `CodedError{code: String, params: serde_json::Value}`, `RunDetails{updated, failed, outdated, manual, error: Option<CodedError>}`
   (all `#[serde(default)]`). `Entry.details: Option<RunDetails>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`.
   Grep every `Entry {` construction (tests, `app.rs`) and add `details: None`. `Outcome` gets `changes: Vec<Change>`, `failed: Vec<Failed>`,
   `manual: Vec<String>` (`Outcome::note` and the four constructors fill defaults; derive `Default` if it is simpler). `AddonsFailed{failed: Vec<Failed>, text}`
   with `Display` = the text and `Error`, like `NotRun`.
2. **CurseForge.** Read the real `AddonGameInstance.json` read-only (names of keys in `installedFile`, e.g. `displayName`, `fileName`; do not
   copy contents into the repo) and keep `(name, file id, file label)` in `Instance.addons`. New pure `changes(before, after) -> Vec<Change>`
   next to `updated` (which stays and is derived from it or left, whichever is smaller; its output is unchanged). `flavor` from `inst.path`'s last
   component when it matches `_[a-z0-9_]+_`. `Outcome.manual` = the existing `manual` list.
3. **WowUp.** Add `version` (`installedVersion`) to `Addon`; `changes(before, after)` for the ones `updated` already finds, `from`/`to` = versions
   (empty -> `None`), `flavor` from `installations()` location's name when it looks like `_name_`. `attempted` becomes `attempted_full(log) ->
   Vec<(name, from, to)>` and `attempted` calls it (same output). `failed` with `reason` by `NO_TOKEN_MARK`. The all-failed `bail!` becomes
   `return Err(anyhow::Error::new(AddonsFailed{..}))` with the identical message text.
4. **Wago.** Keep the text `updated`; add `changes` with `kind` `Weakauras`/`Ui Packs` and `name` = `WagoAppCompanion` / `WagoUI_Storage` (use the
   existing folder constants in `wago.rs`; check the names there).
5. **ElvUI.** Split `update` into the existing I/O and a pure `plan(flavor, versions, latest) -> Option<Change>`; `update` returns `Result<Option<Done{text, changes}>>`
   (callers: `app.rs`, the ignored `live_tukui`). Text identical.
6. **`app::update_addons`.** Keep the `Outcome`: `app.update(..)` -> map to `(summary, RunDetails)`; `record(title, &Result<String>, Option<RunDetails>)`: for an `Err`,
   `details.error = errors::classify(e)` and, when `e.downcast_ref::<AddonsFailed>()`, its `failed`; for `NotRun` the same with an empty list.
   ElvUI row: `details.updated` = its changes. Pre-flight failure row (`no addon app`, a busy lock): `details` with `error` only. The returned `Result<String>` is unchanged.
7. **`errors::classify`**: `NotRun` -> `not_run` (new constant `NOT_RUN`, no params); add to the table in `docs/cli-json.md`.
8. **`cli_json::update_data`**: add per entry `updated`, `failed`, `outdated`, `manual`, `error` (from `details`; `null` each when `details` is `None`). `ts()` for none.
   Existing keys unchanged. Check 0007's `update_data` test and extend it.
9. **Docs.** `docs/cli-json.md` `update-addons` section (example line, field table, `not_run`, delete "not available as data yet"); `CHANGELOG.md` Added/Changed under
   `[Unreleased]` ("`update-addons --json` lists updated and failed addons"; the history file gains an optional field). `docs/arch/overview.md`: only if it describes `Entry`.
10. **Tests** (inline): as listed in the task; fixtures are small JSON/text strings in the test modules. `errors` tests get the Russian-language run.
    No test calls `update` of any manager.
11. **Verify** (PATH from CLAUDE.md): the gates; `ui-snapshot addons` and `addons-skipped` PNGs from base vs new exe compared byte for byte (scratch `--target-dir`,
    never the running exe); the ignored `real_file` tests read-only; `update-addons --json` is not run live (it would start a manager: never).

## Files likely touched
`crates/core/src/{updater,app,curseforge,wowup,wago,elvui,errors}.rs`, `src/cli_json.rs`, `docs/cli-json.md`, `CHANGELOG.md`.
Security-shaped paths touched: `curseforge.rs`, `wowup.rs`, `wago.rs`, `elvui.rs`: no behaviour change allowed (data extraction only; the forced-switch and
swap logic stay as they are). `src/tray.rs`, `src/panel.rs` and the locales are untouched; if the new `Entry` field forces a change there it is a constructor fix only.

## Tradeoffs considered
- **Replace `Outcome.updated: Vec<String>` by `Vec<Change>`** rejected: the summary text and the tests that pin it would churn; new fields next to the old ones keep the
  diff reviewable (the 0007 approach).
- **One `details` object vs flat optional fields on `Entry`**: one object, one `Option`, and "unknown" (`None`) versus "empty" is a single test.
- **Failed list on the error (`AddonsFailed`) vs turning all-failed into an `Ok`**: an `Ok` would change the tray's error balloon and the retry policy; a typed error is the
  existing `NotRun` pattern and changes nothing for callers that format `{e:#}`.
- **Version strings from the apps' own files only**: CurseForge's API and WowUp's release notes need keys/network; reading files is what these modules already do.
- **Naming Wago's WeakAuras** rejected: the app's writes are blobs the module deliberately does not parse.
- **Migrating old rows** rejected: the data does not exist for them.
