# 0007 JSON output for the CLI (the API for a second shell)

## Goal
A global `--json` flag. With it every command a second shell (Overwolf / ow-electron, ADR 0002) needs prints exactly one
JSON object on stdout and nothing else, errors included (as JSON, non-zero exit code). Without `--json` nothing changes:
text output stays byte for byte the same. The contract is written down in `docs/cli-json.md`.

## Why
ADR 0002 chose "the CLI with JSON output" as the way for a non-Rust shell to reach the core (process boundary keeps the
WoW-folder lock and crashes contained, no napi). The JSON itself was left to its own task: this one. Settings get/set is
task 0008 (its own PR, after this one).

## Decisions (made here)
- **Envelope.** Every object has `"v":1` and a `"type"`:
  - success: `{"v":1,"type":"result","command":"backup","data":{...}}`
  - failure: `{"v":1,"type":"error","command":"backup","error":{"code":"busy","params":{...},"detail":"..."}}`,
    optionally with `"data"` beside `error` (a partly done run, see `backup`).
  `type` exists so progress lines (`"type":"progress"`) can be added later without a version bump.
- **Versioning.** New fields and new codes are not a breaking change; removing or renaming a field, changing a type or a
  meaning bumps `v`. A shell must ignore unknown fields. `v` is the same for the whole CLI, not per command.
- **No localized text.** Data are codes, ids, numbers, RFC 3339 timestamps (local time with offset, e.g.
  `2026-11-27T10:00:00+03:00`), paths as strings, flavors by folder name (`_retail_`; titles are for the shell to
  translate), enums as lowercase snake_case strings. Under `--json` the process language is forced to English
  (`i18n::set(Some(Lang::En))`) so that the one free-text field, `error.detail` (the `{:#}` chain of the error), is stable
  English for logs and bug reports. Documented as "diagnostic, never show it, never parse it".
- **Errors have a stable `code` and structured `params`**, derived from the error, not from its text (see Error codes).
  stderr keeps the log lines (env_logger already writes to stderr; English under `--json`); a shell ignores them or keeps
  them for diagnostics. stdout carries only the one object. A panic prints to stderr, exit 101, no JSON: a shell treats
  "no JSON on stdout" as a crash.
- **Exit codes.** 0 success; 1 any error; 2 `invalid_args` (clap's own code). `quit` keeps its 3 (not JSON).
- **Nothing interactive.** `restore --json` without `--yes` fails with `confirmation_required` (exit 2) before touching
  anything; stdin is never read in JSON mode.
- **Dates and sizes.** Timestamps are RFC 3339 with the local offset (snapshot ids stay the local `YYYYmmdd-HHMMSS[-label]`
  strings). Sizes are bytes (integers).
- **Serialize core types directly** (`derive(Serialize)` + a `serialize_with` for timestamps) where the type has no
  persisted format: `snapshot::Snapshot`, `snapshot::Kind`, `chars::Character`, `macros::{Scan, Finding, Kind, Level, How}`,
  `macro_fix::{MacroFix, Variant}`, `maintenance::Found`. Types that are already stored on disk in another shape
  (`profiles::Meta` in `profile.json`, `updater::Entry` in `addon-updates.json`, `Config`) are not touched: a small
  response struct in the shell, or `Config` as it is (its serde names are the contract anyway).
- **Inventory** (which commands get JSON):

  | Command | JSON | `data` |
  |---|---|---|
  | `status` | yes | `app{version}`, `config{path,exists}`, `backups{dir,keep}`, `wow{root,source:"config"|"detected"|"battlenet"|"none",error?}`, `flavors[{name,version,dir,selected,characters,series{wtf,addons}{count,last}}]`, `games[{name,dir}]`, `addon_apps[{app,chosen}]`, `addon_updates{enabled,app,last_run}` |
  | `init` | yes | `config_path`, `config` (the whole Config); an existing config is the error `config_exists` |
  | `backup` | yes | `created[{flavor,kind,id,size,created}]`, `unchanged[{flavor,kind}]`, `skipped_game` (name or null), `failures[]`; failures make it an error `partial` (exit 1) with the same object in `data` |
  | `list` | yes | `series[{flavor,kind,snapshots[{id,label,created,size,path}]}]`, newest first as in text |
  | `restore` | yes | `flavor`, `plan[{kind,snapshot}]`, `applied:true` (`--yes` required) |
  | `chars` | yes | `flavors[{flavor,characters[{id,account,realm,name,path}]}]`, `id` = `ACCOUNT/Realm/Name`, the form `--from`/`--to` accept |
  | `macros` | yes | `checked`, `missing[flavor]`, `levels{broken,warn,info}`, `findings[...]` (the `Finding` fields, `fixed` = variants with `lang,text,chars,marks,check,over,rest`), `names_updated` with `--update-names` (its progress lines go to the log, not stdout) |
  | `copy-char` | yes | `dry_run`, `source`, `targets[{character,files[]}]` |
  | `update-addons` | yes | `ran`, `entry{time,app,ok,manual,skipped,detail}` of the new history row (`detail` = the diagnostic text of that run); structured updated/failed addon lists only where `updater::Outcome` already carries them (see plan) |
  | `clean` (no items) | yes | `items[{key,files,bytes,names[]}]`, `big[{name,bytes}]` |
  | `clean <items>` | yes | `results[{key,files,bytes,failed,skipped_flavors[]}]` |
  | `outdated` | yes | `flavors[{name,version,judged,addons[]}]` |
  | `profile list` | yes | `profiles[{id,number,flavor,account,realm,name,created,files,addons[]}]`, `id` = `<flavor>/<zip file name>` (stable; `number` shifts when one is deleted) |
  | `profile save` / `apply` / `delete` | yes | `save`: the profile as in `list` + `unchanged`; `apply`: `profile`, `character`; `delete`: `deleted:id`. `apply`/`delete` accept the number or the `id` |
  | `self-update` | no | a shell updates itself; `--json` gives `json_unsupported` |
  | `schedule *` | no | same: the shell does its own autostart; `json_unsupported` |
  | `tray`, `watch`, `quit`, `ui-snapshot` | no | long-running or windowed; `json_unsupported` (exit 2). A shell that wants automatic backups runs `watch` (its stdout stays unused, log on stderr) or schedules `backup` itself |

- **Error codes** (`error.code`; `params` in brackets): `busy` [flavor, pid, operation: the holder from the lock's `.info`,
  null when unknown], `game_running` [game, flavor], `not_found` [what: `snapshot|character|profile|flavor`, key],
  `invalid_args` [kind, and `detail` = clap's text], `confirmation_required`, `no_wow`, `config_invalid` [path],
  `config_exists` [path], `io` [kind: `std::io::ErrorKind` lowercased, os_code], `network` [status?], `partial`,
  `json_unsupported`, `failed` (anything unclassified; `detail` only). New specific codes may be split off `failed` later.
- **Progress: final result only in v1.** Operations take seconds to a minute, the tray has its own progress; a second
  shell can show a spinner. The envelope is ready for it (`type`), and `snapshot::progress`/`on_progress` exist, so
  NDJSON lines (`{"v":1,"type":"progress","what":"_retail_ wtf","done":..,"total":..}` before the final line) can come as
  a flag-compatible follow-up (`--json-progress`) when a shell asks for it. One line of recommendation: not now.
- **Split.** Config get/set is task 0008 and its own PR; this task is one PR (large but mechanical; commits per step in the plan).
- **ADR.** The envelope and the versioning rule are a public contract; ADR 0002 left "the JSON schema" undecided and ADRs
  are immutable, so this PR also adds ADR 0003 (envelope, versioning, error codes, progress later) next to
  `docs/cli-json.md`, and updates `docs/arch/overview.md`.

## Scope
- `--json` global flag; JSON paths for the commands in the inventory; JSON clap errors (the flag is detected before clap
  parses, like `early_language`); `json_unsupported` for the rest.
- A coded error type in the core (one struct, not an enum per case) and its use in the places that matter: the lock, the
  game-running check, "not found" lookups, a bad config, `http::get`.
- Structured results where a core function returns only text today (`backup_all` report, `maintenance::clean`, the
  profile save/apply, `update_addons`): add fields/variants next to the text, text unchanged.
- `docs/cli-json.md` with an example per command and the error table; `docs/arch/overview.md` ("Shells"); `CHANGELOG.md`
  `[Unreleased]`; a short "JSON output" paragraph in `README.md` and the four translations; the `cli.json` help key in five
  locales; `CLAUDE.md` Layout line for the new module.
- Tests: see Acceptance.

## Out of scope
Config get/set (0008); NDJSON progress; JSON for `self-update`, `schedule`, `tray`, `watch`, `quit`, `ui-snapshot`; richer
character data for a shell (class, faction, last played from `profiles::characters`); reading the macro report saved by the
tray; a native binding; changing any text output; a version bump.

## Acceptance
- Gates (root and core): `cargo fmt --all --check`, `cargo clippy --workspace --release --all-targets --locked -- -D warnings`,
  `cargo test --workspace --release --locked`. `site/` untouched.
- Every text command prints the same bytes as before on a scratch config and a fake tree (`status`, `list`, `chars`,
  `outdated`, `clean`, `profile list`, `macros`, `backup` into a scratch `backup_dir`; compared before and after). Only
  `--help` of the root and its language keys change (the new option).
- `--json` stdout is exactly one line of JSON (`serde_json::from_str` of the whole stdout works) for each command of the
  inventory, with `v:1`; no `println!` of a text path runs in JSON mode; core has no `println!` (grep).
- Snapshot tests, one per JSON command, in `src/` inline tests: fake WoW tree (`tempfile`) + scratch config, run the JSON
  function, normalize (timestamps -> `<ts>`, snapshot ids -> `<id>`, the temp root -> `<root>`; `games` and `addon_apps`
  of `status` only checked for their type: they read the real machine) and compare with a plain `serde_json::json!`
  expectation. No new dev-dependency (`insta` rejected: ~15 expected objects do not justify it).
- Error tests: a held lock gives `{"type":"error","error":{"code":"busy","params":{"flavor":"_retail_","pid":<own pid>,"operation":"..."}}}`
  and exit code 1 (core test with `acquire_in`; one shell-level run in-process); `game_running`, `not_found` (snapshot,
  character), `invalid_args` from clap (`--json restore --bogus`), `confirmation_required`, `json_unsupported`, `config_invalid`.
- The classifier never reads `tr!` text: a test builds each error with the language set to Russian and gets the same code
  and params.

## Open questions: decided (owner)
1. `error.detail` as forced-English diagnostic text: **accepted**. The shell must never show it or parse it.
2. `profile apply/delete` accept a number or the stable id: **yes**.
3. `self-update`, `schedule`, `tray`, `watch`, `quit`, `ui-snapshot` answer `json_unsupported`: **yes**.
4. ADR 0003 for the JSON contract: **yes**, written in this PR (`docs/adr/0003-cli-json-contract.md`).
5. Progress: the final result only in v1; NDJSON progress later, behind a flag.
6. Version to ship in: left to the owner (no bump here; changelog under `[Unreleased]`).
