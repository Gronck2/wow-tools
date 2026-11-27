# Plan 0007

One PR, commits per step; the tree builds and tests pass after each. The text paths are never edited except to add
fields next to the text; JSON lives in new code.

## Facts read
- `main.rs::run` dispatches to `cmd_*` functions that `println!` localized text. Logging: `env_logger` default target is
  stderr (the console branch of `init_logging`), the file branch only for the tray / `--log-to-file`. So the log never
  pollutes stdout. Prints that do reach stdout: the `cmd_*` functions, `confirm()`, and the `macro_names::update_for`
  callback in `cmd_macros`.
- Errors are `anyhow` with `tr!` text; `updater::NotRun` is the existing precedent for a typed error found by downcast.
  `lock::acquire_in` bails with `app.busy` text built from `fl.title()` and the `.info` line (`<pid> <operation>`).
  `app::backup_all` turns errors into `Report.errors: Vec<String>` and `Report.created: Vec<String>` (text lines only).
- `maintenance::clean`, `profiles::save/apply`, `app::update_addons` return text. `Snapshot.created` is a
  `NaiveDateTime` (local). `Config` is `Serialize` and `serde(default)`.

## Steps
1. **Coded errors in core** (`crates/core/src/errors.rs`, `pub mod errors`):
   `pub struct Coded { pub code: &'static str, pub params: serde_json::Value, message: String }` implementing `Display`
   (= `message`, so text output is identical), `std::error::Error`, with constructors `busy`, `game_running`, `not_found`,
   `config_invalid`, `config_exists`, `no_wow`, `confirmation_required`; code constants in one place.
   `pub fn classify(&anyhow::Error) -> (code, params)`: walk `err.chain()`; first `Coded` wins; else a `std::io::Error` ->
   `io {kind, os_code}`; else a `http::NetError` -> `network`; else `failed`. Then adopt it where the text is born:
   `lock::acquire_in` (the `.info` is parsed into `pid`/`operation`, the message string is built exactly as now; the
   existing lock tests keep passing), `app::ensure_game_closed`, `snapshot::select`/`chars::find`/`profiles` "no such"
   lookups, `Config::load` (bad TOML), `wow::resolve_root` failure, `cmd_init` exists check. `http::get` errors get a
   small `NetError { status: Option<u16> }` context. Unit tests in core: each constructor classifies right with the
   language set to Russian; a `Coded` under `.context(...)` is still found.
2. **Structured results next to the text.**
   - `app::Report`: add `snapshots: Vec<Created{flavor, kind, id, size, created}>`, `unchanged: Vec<(flavor, kind)>`,
     `failures: Vec<Failure{flavor, kind: Option<Kind>, code, params, detail}>` filled where `created`/`errors` are filled
     (a busy flavor goes through `classify` on the lock error). The tray keeps using the old fields.
   - `maintenance`: split `clean` into `clean_item(ctx, Item) -> Cleaned{files, bytes, ...}` and the existing text
     formatter calling it (read the function first: use whatever it counts for the `cleanup.removed` text).
   - `profiles::save/apply`: expose the saved `Meta` / the target (they build `summary` from them already).
   - `app::update_addons`: return the structured `Outcome` data it already has per app (`updater::Outcome`, history
     `Entry`) next to the String; if names are not available per addon, v1 `data` stays `entry` with `detail` only and the
     doc says so.
   - `derive(Serialize)` on `snapshot::{Snapshot, Kind}` (`rename_all = "lowercase"`), `chars::Character`
     (+ `id` method), `macros::{Scan, Finding, Kind, Level, How}` (`tag = "type"`, snake_case), `macro_fix::{MacroFix, Variant}`,
     `maintenance::{Found, Item}`; timestamps through one helper `errors`-sibling `ser::local_rfc3339` (`from_local_datetime(..).earliest()`,
     `to_rfc3339()`). `Arc` fields need serde's `rc` feature on core's `serde` dependency (check `Cargo.lock` stays valid
     with `--locked`). **Check `Finding.scope`**: for account macros it is built with `tr!("macros.account", ..)`: that is
     localized text; serialize it as the account id only (the `account: true` flag exists), or `skip` it and add
     `account_id`. Verify no other `tr!` result is stored in a serialized field (`rg 'tr!' crates/core/src/macros.rs`).
     Do not touch persisted formats (`macro_store`'s file keeps its own shape; if `Finding` is already serialized there, add
     the new attributes only if the stored bytes do not change: its tests prove it, otherwise use a response struct).
3. **JSON layer in the shell** (`src/cli_json.rs`, `mod cli_json;`):
   - `Envelope`, `ok(command, data)`, `err(command, &anyhow::Error)` using `errors::classify`; `detail` = `format!("{e:#}")`.
   - `pub fn run(ctx: &Ctx, cmd: &Cmd) -> Result<serde_json::Value>`: one small function per command in the inventory,
     calling the same core functions as `cmd_*` and returning `data`. `status` reuses `Ctx`, `wow`, `chars::list`,
     `Store::list`, `wow::running_games`, `updater::{chosen, AddonApp::installed}`; no text. `restore` requires `yes`
     (`Coded::confirmation_required`). `macros --update-names` passes a callback that only `log::info!`s.
   - `profile apply/delete`: the argument becomes `String`; digits = number as today, anything else matches the zip file
     name; the text path calls the same resolver, so numbers behave identically.
   - `print(value) -> exit_code`: one `println!` of the compact JSON, flushed, `BrokenPipe` ignored. Exit 0/1/2.
4. **Wire it** in `main.rs`: `#[arg(long, global = true)] json: bool` (help key `cli.json`); `early_json()` scans args
   like `early_language` (also `--json=true` is not supported: plain flag only). If JSON: `i18n::set(Some(Lang::En))`
   instead of the config language; use `try_get_matches()` so a clap error prints the envelope `invalid_args` (kind =
   `ErrorKind` lowercased, `detail` = rendered text without ANSI), exit 2; `--help`/`--version` (`DisplayHelp`,
   `DisplayVersion`) are printed as usual. `run` branches: `if cli.json { return json_main(cli) }` before the `match`;
   `Tray/Watch/Quit/UiSnapshot/SelfUpdate/Schedule` give `json_unsupported` there. `Ctx::load` failure under JSON is also an
   envelope. No other line of `cmd_*` changes.
5. **Locales:** `cli.json` in `crates/core/locales/{ru,en,de,fr,es}.toml` (ru first). Nothing else is translated: JSON has no
   text. The help-key test and the key-parity test cover it.
6. **Tests** (inline, bottom of `src/cli_json.rs`; classifier tests in `crates/core/src/errors.rs`):
   - helper `fake_tree()` -> (`tempfile::TempDir`, scratch config path) with `<root>/_retail_/{WTF/Account/ACC/Realm/Name/..,Interface/AddOns/X/X.toc}`,
     `backup_dir` in the temp dir, `wow_path` set; reuse any existing fake-tree helper in `app.rs` / `snapshot.rs` tests (grep first).
   - `normalize(&mut Value, root)`: replace RFC 3339 strings (regex-free: parse with `chrono::DateTime::parse_from_rfc3339`) by `"<ts>"`,
     ids matching `^\d{8}-\d{6}` by `"<id>"`, and the root prefix (both slash styles) by `"<root>"`; drop `games`/`addon_apps`
     after asserting they are an array.
   - one test per command: `status`, `init`, `backup` (twice: created, then unchanged), `list`, `restore` (with and
     without yes), `chars`, `macros` (tree with a `macros-cache.txt` with a known-bad line), `copy-char` (dry run and real),
     `clean` scan and one item, `outdated`, `profile save/list/apply/delete`; `update-addons` only its `ran:false`/refusal
     path (no real manager runs in tests: never touch addon apps). Expected values are `json!` literals.
   - errors: busy (hold `lock::acquire` on the fake flavor in the test, run the `backup` and `clean` JSON functions; the lock
     files land in the real `%LOCALAPPDATA%\WoW Tools\locks` under a hash of the temp path: harmless, noted in the test),
     `game_running` via a pure function test of the classifier (a real game cannot be started), `not_found` x2,
     `confirmation_required`, `json_unsupported`, `config_invalid` (bad TOML), clap `invalid_args` through
     `Cli::command().try_get_matches_from([...])`.
   - a test that `ok()`/`err()` output has no `\n` and parses as one value.
7. **Docs:** `docs/cli-json.md` (English): invocation, envelope, versioning rule, exit codes, stderr note, error code table
   with params, one real example per command (generate them from the scratch-config run, anonymize), the "detail is
   diagnostic" note, the `restore --yes` rule, how a shell takes the lock (it does not: it calls the CLI). `docs/arch/overview.md`
   "Shells" section: the JSON CLI exists, link. `CLAUDE.md` Layout: one line for `cli_json.rs` and `errors.rs`; the README
   mention. `CHANGELOG.md` `[Unreleased]`: "Added: `--json` for the command line (for other front ends); see docs/cli-json.md".
   README.md + `README.{ru,de,fr,es}.md`: two sentences in the CLI section, kept in step; no picture changes.
8. **Verify** (PATH from CLAUDE.md): gates; before/after byte comparison of text output with a scratch config on the fake
   tree (build the base commit's exe into a scratch `--target-dir`, never replace `target\release\wow-tools.exe`, never stop
   the tray); run each `--json` command against the scratch config and pipe through a JSON parser to see exactly one line
   (`uv run python -c`); `grep -rn "println!" crates/core/src` is empty outside tests; `ui-snapshot` not needed (no UI change).
   Real install: only read-only commands with `--config <scratch>.toml` (`status --json`, `chars --json`, `list --json`).

## Files likely touched
`src/main.rs`, new `src/cli_json.rs`, new `crates/core/src/errors.rs`, `crates/core/src/{lib,lock,app,snapshot,chars,profiles,maintenance,macros,macro_fix,wow,http,config,updater}.rs`,
`crates/core/Cargo.toml` (serde `rc`), `crates/core/locales/*.toml`, `docs/cli-json.md`, `docs/arch/overview.md`, `CLAUDE.md`,
`CHANGELOG.md`, `README*.md`. Security-shaped paths touched: `src/main.rs`, `src/config.rs` (core's `config.rs` only for the
typed error), `src/snapshot.rs`: reviewer attention there; no behaviour change allowed.

## Tradeoffs considered
- **JSON by threading a `json` flag through every `cmd_*`** (one code path, text and JSON from the same data) rejected: it
  touches every print and risks the byte-for-byte rule; a separate thin JSON layer over the same core calls duplicates
  only the glue (flavor selection is already in `Ctx`).
- **A typed error enum per case** vs one `Coded` struct: the struct is one place, any module can raise a code without a
  dependency on a central enum, the `Display` keeps the localized text for the tray and the CLI. The enum would be nicer
  for matching in Rust; no Rust caller needs that yet (`NotRun` stays as it is).
- **Code only, no `detail`** rejected (see Open question 1). **Localized `detail`** rejected by the owner's rule.
- **`insta` snapshots** rejected: new dev-dependency, `.snap` files to review; plain `json!` expectations in the test are
  readable and need no tooling.
- **NDJSON progress now** rejected (see the task); the `type` field keeps the door open without a version bump.
- **Core owns `classify`** (not the shell): the busy test then runs in the core with its temp lock dir, and a Rust second
  shell gets the same codes.
- **Process-level tests** (spawning the exe) rejected: the repo has no `tests/` dir by contract and the in-process functions
  return the same `Value`; the step-8 manual runs cover the process boundary.

## Deviations found while implementing
- The JSON is built by hand (`json!`) in `src/cli_json.rs`, not by `derive(Serialize)` on core types (so no serde `rc`
  feature): `macros::Finding`/`Scan`/`Kind` are already stored in `macro-report.json`, and `scope` and the alias labels
  carry interface-language text. `Finding` got a `serde(skip)` `owner` (flavor folder, account, realm, character) instead;
  the alias labels are split into `{character, locale, guessed}` in the shell.
- `http::get` errors are `Coded` `network` with `host` only (the function fails before there is a status).
- `clean <items>` results are `files, bytes, failed, skipped_flavors` (no text `detail`); `maintenance::clean_item` returns
  `Cleaned` and `clean` formats it.
- Profile `id` is `<flavor>/<zip file name>` (a bare file name could repeat in two flavors); `Profile::id` is in the core.
- `main::copy_char` takes a callback per target; `cmd_copy_char` prints from it (the text is unchanged).
- `update-addons` has no end-to-end test: it would start the real addon manager of the machine. The data shaping is tested
  (`update_data`).
- The tests' lock files go to the real `%LOCALAPPDATA%\WoW Tools\locks` (empty files named by a hash of the temp path).
- The `--json` help text is the one text change of `--help` (every command lists the global option), plus the profile
  argument help ("number or id").
