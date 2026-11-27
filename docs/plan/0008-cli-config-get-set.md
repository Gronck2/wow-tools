# Plan 0008

Starts after 0007 is merged (uses `src/cli_json.rs`, `errors::Coded`, the envelope). One PR.

## Steps
1. **Core helpers** in `crates/core/src/config.rs` (a small `impl Config` block, tests inline):
   - `Config::to_image(&self) -> serde_json::Map` via `serde_json::to_value`; with `preserve_order` the key order is the
     struct's. `None` fields appear as `null`, so the image has every key.
   - `Config::get_key(&self, key) -> Result<Value>`: unknown key = `Coded::not_found("config_key", key)`.
   - `Config::set_key(&mut self, key, value: Value) -> Result<()>`: clone the image, replace the key, `serde_json::from_value::<Config>`
     (error -> `invalid_args {key, reason:"type"}` with the serde error text as `detail` only), then `validate()`
     (time formats, `keep`, `*_every*`, flavor names) -> `invalid_args {key, reason:"range"|"format"}`; assign on success
     (whole struct, so no half state).
   - `Config::unset_key(&mut self, key)` = `set_key(key, default image value)`.
   - `fn parse_value(raw: &str) -> Value`: `serde_json::from_str` else `Value::String`. Careful: a bare `10:00` fails JSON and
     stays a string (right for `backup_time`); a bare `true` becomes a bool (right); a path like `D:\Shots` fails JSON and
     stays a string; `"123"` for a string-typed key would parse as a number and be rejected by the type check, which tells the
     user to quote it: document in `docs/cli-json.md`.
   - `Config::save`: `fs::write` to `<name>.tmp` in the same folder, then `fs::rename` over the target (Windows `rename`
     replaces an existing file with `std::fs::rename`); on error remove the temp. `create_dir_all` as now.
2. **CLI** in `src/main.rs`: `Cmd::Config { action: ConfigCmd }` with `Get { key: Option<String> }`,
   `Set { key, value }`, `Unset { key }`; the help doc comments (English) and `cli.config.*` keys in five locales
   (`about`, `get.about`, `get.key`, `set.about`, `set.key`, `set.value`, `unset.about`, `unset.key`); text printing:
   `get` = `toml::to_string_pretty(&cfg)` or the one value as TOML/JSON-ish plain text (string without quotes, the rest as JSON), one
   string for the text modes in `out.config_set` ("{key} = {value}") in five locales. Loading uses `ctx.cfg` and `ctx.config_path`
   (an explicit `--config` is honoured); `set`/`unset` operate on a freshly loaded `Config::load(&ctx.config_path)`.
3. **JSON** in `src/cli_json.rs`: `config` get/set/unset data as in the task; `defaults` from `Config::default()` image.
   Un-JSON'able nothing here.
4. **Tests** as in the task's Acceptance, in core `config.rs` and shell `cli_json.rs` tests. The save test checks that a
   crash between temp write and rename leaves the old file valid (simulate by a read-only target dir is flaky on Windows: only
   assert the temp file is gone and the content is whole).
5. **Docs:** `docs/cli-json.md` "config" section (examples, the quoting rule, "close the settings window first");
   README x5 one sentence; `CHANGELOG.md` `[Unreleased]`; `CLAUDE.md` Layout mention in the `config.rs` line.
6. **Verify:** gates; text of every command from 0007 unchanged; on a scratch config run `config get --json`,
   `config set keep 5 --json`, `config set keep 0 --json` (error, file unchanged), `config unset update_app --json`,
   then `wow-tools list --config <scratch>` still loads it. Never `--config` the real file.

## Files likely touched
`crates/core/src/config.rs`, `crates/core/src/errors.rs` (a code or two), `crates/core/locales/*.toml`, `src/main.rs`,
`src/cli_json.rs`, `docs/cli-json.md`, `README*.md`, `CHANGELOG.md`, `CLAUDE.md`.

## Tradeoffs considered
- **JSON image round trip** (config -> `Value` -> patch -> `Config`) vs a hand-written match per key: the round trip reuses
  serde and the existing `known_tabs` check, needs no table to keep in step with new fields, and a new `Config` field works
  at once. Cost: error messages come from serde (diagnostic `detail` only) and all-key validation lives in `validate()`.
- **`toml_edit`** (comment-preserving) rejected: new dependency, and the settings window already rewrites the file whole.
- **Generic `config set` taking TOML** rejected: the shell speaks JSON, one syntax to document.
- **Locking the config file** rejected: the atomic rename covers readers; two writers are last-wins, as today.

## As implemented (deviations from the plan above)
- 0007 is merged: JSON goes through `cli_json`'s envelope; a refused value is `invalid_args` with params `{kind:"invalid_value", key, reason}`
  (`Coded::invalid_value`, exit 2), an unknown key `not_found {what:"config_key"}`. Documented in `docs/cli-json.md`.
- The top-level `--config` help key moved from `cli.config` to `cli.config_file`: TOML cannot hold both the string and the `[cli.config]` table.
- The shared set/unset path is `config_change` in `src/main.rs` (text and JSON both call it); validation is `Config::invalid` in core.
- Text `get` prints the TOML (`toml::to_string_pretty`), `get <key>` the bare value, `set`/`unset` the `config.set_line` locale line.
