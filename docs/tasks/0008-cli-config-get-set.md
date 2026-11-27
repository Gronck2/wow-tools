# 0008 `config get` / `config set` in the CLI

## Goal
Read and change the settings through the CLI so a second shell never edits `config.toml` itself:
`wow-tools config get [key]`, `config set <key> <value>`, `config unset <key>`, text and `--json` (envelope of task 0007).
Keys are the serde field names of `Config` (`keep`, `backup_when`, `backup_days`, ...), the same names as in the file.

## Why
Task 0007 gives a second shell the operations; it still needs the settings (what to back up, when, which addon manager).
Parsing and writing TOML in another language would duplicate the validation and could write a file the Rust side rejects.
Own PR, after 0007 (it uses its envelope, error codes and `cli_json` module).

## Decisions (made here)
- **Commands.**
  - `config get` — whole config; `--json` data: `{"path","exists","config":{...all keys...},"defaults":{...}}`
    (`defaults` lets a shell draw "reset"). Text: the TOML as `init` prints it.
  - `config get <key>` — one value; JSON `{"key","value","default"}`; unknown key = `not_found {what:"config_key"}`.
  - `config set <key> <value>` — validates, saves, prints the new value (JSON `{"key","value","changed":bool}`).
  - `config unset <key>` — back to the default for that key (`null` for the optional ones: `wow_path`, `update_app`, `language`, `screenshots_dir`).
  - Hidden/not offered: `config path` (it is in `get`), a multi-key patch (see Open questions).
- **Value syntax.** The `<value>` argument is parsed as JSON; if that fails it is taken as a plain string. So
  `config set keep 10`, `config set update_addons true`, `config set backup_when game_exit`,
  `config set backup_days '["mon","fri"]'`, `config set screenshots_dirs '{"_retail_":"D:\\Shots"}'`,
  `config set language null`. The result is put into the config's JSON image, deserialized back into `Config`
  (so every enum, `known_tabs`, `PathBuf` and range the type already checks is checked by the same code the file load uses).
  A key not in the config's JSON image is rejected before that.
- **Extra validation** the type does not do (added next to `Config`, so the settings window can reuse it): `backup_time`,
  `update_time` parse as `HH:MM`; `keep >= 1`; `backup_every_days >= 1`; `update_every >= 1`; flavors, if given, look like
  `_name_`. Failure = `invalid_args {key, reason:"type"|"range"|"format"}` (exit 2), nothing written.
- **Writing.** Same as the settings window: `Config::save` rewrites the whole file with `toml::to_string_pretty` (comments
  in a hand-edited file are lost: documented, same as every click in the settings window today). Change: `save` writes a temp
  file in the same folder and renames it, so the tray's loop never reads half a file (a small, safe improvement that also
  helps the settings window). No lock: the config is not a WoW folder.
- **No push to a running tray.** The tray's loop re-reads the config every round; the settings window holds its own copy and
  overwrites the file on its next click. Documented in `docs/cli-json.md`: change settings while the settings window is
  closed. Out of scope: notifying the tray.
- **`language`** accepts the `Lang` serde names (`ru`, `en`, ...) or null; setting it does not change the `--json` output
  (always English `detail`).
- **Security-shaped:** `src/config.rs` and `src/main.rs` are on the list; the setter can point `backup_dir`, `wow_path`,
  `screenshots_dir` anywhere the user (or the shell running as the user) can: no new privilege, but the reviewer should
  look at it.

## Scope
`config` subcommand (clap, help keys in five locales), `Config::set_key`/`unset_key`/`get_key` helpers in core
(`serde_json::Value` image in/out + the extra checks), atomic `Config::save`, JSON and text paths, tests, docs
(`docs/cli-json.md` section, README mention in five languages, CHANGELOG `[Unreleased]`, CLAUDE.md layout line).

## Out of scope
A settings schema with descriptions/enum lists for a form UI (the shell knows its own forms; `defaults` is enough),
a multi-key transaction, notifying the tray, preserving TOML comments (`toml_edit` rejected: new dependency for a
cosmetic gain), a change of the settings window.

## Acceptance
- Gates as in 0007.
- Core tests: set/get/unset round trip for one key of each kind (bool, number, string, enum, `Vec<Day>`, `Option<PathBuf>`,
  `BTreeMap`, `Vec<PanelTab>`); unknown key, wrong type, bad enum value, `backup_time` "25:00", `keep` 0 are rejected and the file
  is byte-identical afterwards; a successful set changes only that key (parse the saved file and compare with the old
  `Config` modified in Rust); a missing config file is created from the defaults; temp file is gone after save.
- Shell tests (inline, scratch config in a tempdir): JSON snapshot of `config get`, `config get keep`, `config set keep 10`,
  an error (`invalid_args`), `config unset update_app`. Text output of `config get` equals `toml::to_string_pretty(&cfg)`.
- All existing 0007 text-output checks still pass; `cli.config.*` help keys exist in five languages (help-key test).
- Real config never touched by tests or checks: `--config <scratch>.toml` only.

## Open questions (owner): decided as recommended
1. Multi-key `config set k1 v1 k2 v2` (all or nothing) for a settings form that saves several fields at once:
   **not now**; sequential single sets are fine and a failed one leaves the earlier ones applied. Add it when a shell needs it.
2. Keep comments via `toml_edit`: **no**; consistent with the settings window.
3. Should `config set` refuse while the tray is busy (e.g. mid-backup)? **No**: the config is read between rounds, a changed
   setting applies on the next one.
