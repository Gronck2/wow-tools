# 0010 Updated addons as data

## Goal
What an addon update did is available as data: which addons were updated (name, from, to, flavor where known), which failed
and why, which are still outdated or left to the user. It is stored in the history (`data\addon-updates.json`, old rows stay
readable) and printed by `update-addons --json` (and, through the same builder, by the `addons` event of `watch --json`, task 0009).
The tray and the panel keep showing the text summary as today.

## Why
`docs/cli-json.md` says "the list of updated addons is not available as data yet": `updater::Outcome.updated` is a list of names that
`app::update_addons` flattens into a localized string before the history row is written. A shell cannot show "BigWigs 1.2 -> 1.3" or
"WowUp could not update X: no Wago token". The managers already know most of it.

## Decisions (made here)
- **Shape** (new optional field `details` of the history row `Entry`; `null`/absent in old rows = "unknown", `[]` = "none"):
  ```
  details: {
    updated:  [{ name, kind: "addon"|"weakauras"|"ui_packs", from: str|null, to: str|null, flavor: str|null }],
    failed:   [{ name, from, to, flavor, reason: "no_token"|"unknown" }],
    outdated: [name],            // still behind after the run
    manual:   [name],            // "update by hand" in the app, left alone (CurseForge)
    error:    { code, params } | null   // why the run failed or was skipped (errors::classify; no text)
  }
  ```
  `Entry` gets `#[serde(default, skip_serializing_if = "Option::is_none")] details`. `Entry` has no other new field, so the file
  stays valid for the previous exe (an unknown key is ignored by serde unless `deny_unknown_fields`: check it does not).
  The history keeps its 20-row cap.
- **What each source can say** (verified in code; the CurseForge fields to be confirmed against the real file read-only):
  - **CurseForge**: `installedFile.id` per `addonID` before/after gives the changed addons by name. `from`/`to` = the file's
    `displayName` (or `fileName`) when `installedFile` has one, else the numeric file id as a string; `flavor` = the instance's folder
    name (`installPath` last component, only if it looks like `_name_`). `outdated` = names whose `latestFile.id` differs;
    `manual` = per-addon `preferenceAutoInstallUpdates:false`.
  - **WowUp / WowUp-CF**: `installedVersion` (human) before/after of addons whose `installedExternalReleaseId` changed (release ids are
    opaque and not shown). `flavor` from the installation's `location` when it is a `_name_` folder, else null. `failed` = the log's
    `[AddonUpdate] provider id Name 'a' -> 'b'` lines that did not end up updated (`from`/`to` are those quoted strings), `reason` =
    `no_token` when the "[wago] no token received" mark is in the log (the Wago-token failure), else `unknown`.
    The all-failed run is an error today (`bail!`): the list must survive it, so it rides on a typed error (`updater::AddonsFailed`, found
    by downcast like `NotRun`) and goes into the row.
  - **Wago App**: it cannot name addons. `updated` = at most two items with `kind` `weakauras` / `ui_packs`, `name` = the folder it wrote
    (`WagoAppCompanion`, `WagoUI_Storage`), `from`/`to` null. (Today's `Outcome.updated` holds a localized sentence for it: the text stays,
    the data is separate.)
  - **ElvUI** (`elvui::update`): per flavor `from` = the installed version, `to` = tukui's version, `flavor` = folder name; a repair of a
    mixed install has `from` = the oldest leftover version (equal to `to` = "reinstalled"). `update` returns text plus these changes.
  - **Skipped** (`NotRun`: manager open with auto-update off): `skipped:true` as today, `details.error.code` = a new code **`not_run`**
    (classifier reads `updater::NotRun`; documented, additive). Refusal while a game runs stays `game_running` without a row.
- **Text unchanged.** The localized summary/`text` of the history row, `Outcome::summary`, the balloons and the panel are not touched:
  new fields sit next to the old ones (the 0007 rule). `Outcome.updated: Vec<String>` stays; `Outcome` gets `changes`, `failed`, `manual`
  (CurseForge's `manual` is a name list already built for the note).
- **`update-addons --json`**: each `entries[]` item gets `updated`, `failed`, `outdated`, `manual`, `error` flattened from `details`
  (all `null` for a row without details, which only happens for a refusal-type row recorded before a manager ran: `app::update_addons`
  records its pre-flight failure with `details` holding the `error` only). `data.ran`, existing keys and meanings do not change
  (additive, no `v` bump).
- **Tray/panel**: unchanged (no new UI, no new locale strings). A trivial improvement is not taken: version numbers would need strings
  in five languages and a layout check.
- **No ADR.** It is an additive field in an existing contract and a file format; the main thread may disagree.

## Scope
- `updater`: `Change`, `Failed`, `RunDetails` types (serde), `Entry.details`, `AddonsFailed` error, `Outcome` fields.
- `curseforge.rs`, `wowup.rs`, `wago.rs`, `elvui.rs`: fill them (pure helpers returning data; I/O unchanged).
- `app::update_addons`: keep the `Outcome` (today it drops it after `summary()`), pass details to `record`; the same rows as before otherwise.
- `errors::classify`: `NotRun` -> `not_run`.
- `src/cli_json.rs::update_data`; `docs/cli-json.md` (the `update-addons` section: shape, the removed "not available as data" sentence,
  error `not_run`), `CHANGELOG.md`.
- Tests (see Acceptance).

## Out of scope
Showing versions or lists in the tray/panel; per-addon history older than the 20 rows; a changelog fetch for addons; naming Wago App's
WeakAuras/UI packs individually; telling apart CurseForge's several instances beyond `flavor`; the `watch --json` stream itself (0009);
migrating old rows (they stay `details: null`).

## Acceptance
- Gates (root and core) as in 0009. `site/` untouched.
- Old history files parse: a fixture row without `details` (the exact current shape, including one without `skipped`) round-trips, shows
  `details: None`, and writes back without a `details` key.
- Pure tests with fixture JSON/text, no manager started and no real install touched:
  CurseForge `parse_instances` + changed-file diff gives `updated` with from/to/flavor, `outdated` and `manual`; WowUp `updated`
  with `installedVersion`, `attempted` with the quoted versions and the `no_token` reason for a sample log (also the all-failed case
  carries the list on `AddonsFailed`); Wago change list for the (old, new) state pairs of the two folders; ElvUI per-flavor `from`/`to`
  and the repair case from a tempdir with `.toc` files (the network fetch is not part of the helper).
- `errors::classify(NotRun)` = `not_run`, same under Russian.
- `update_data` shape test with rows that have and lack `details`; whole `json!` literals.
- Existing text tests (`Outcome::summary`, `elvui` history text, wowup `attempted`) pass unchanged; the panel history text is unchanged
  (`ui-snapshot addons` renders identically before/after: compare the PNG bytes of `addons` and `addons-skipped` made by the base and the
  new exe; no UI diff expected).
- The ignored real-file tests (`real_file` for CurseForge/WowUp) may be run read-only to confirm the `installedFile` fields; their result is
  noted in the PR.

## Open questions (owner), with recommendation
1. **CurseForge versions**: if `installedFile` has no display name in the real file, `from`/`to` are numeric file ids (useless to a
   person). Recommend still shipping them as strings (the shell can show the name only), and not calling CurseForge's API (needs a key,
   rejected in the project already).
2. **Failed ones also when the whole run succeeded for other addons**: yes (`failed` filled while the row is `ok`). Recommend yes.
3. Flavor for WowUp may be null for odd install locations: accept. Recommend yes.
