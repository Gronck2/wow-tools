# 0003. The JSON contract of the command line

Status: accepted (2026-11-27)

## Context

ADR 0002 chose "the CLI with JSON output" as the way for a shell in another language (Overwolf, Electron) to reach the
core, and left the JSON itself open. It is a public contract: a second program will parse it and ships on its own
schedule, so it must be versioned and must not depend on the interface language.

## Decision

- **One object per call.** `wow-tools --json <command>` prints exactly one line of JSON on stdout, errors included;
  stderr keeps the log. No JSON on stdout means a crash.
- **Envelope.** Every object has `"v":1` and a `"type"`. Success: `{"v":1,"type":"result","command":..,"data":{..}}`.
  Failure: `{"v":1,"type":"error","command":..,"error":{"code","params","detail"}}`, with `data` beside `error` when a
  run did part of its work. Exit code 0 / 1 / 2 (2 = wrong use: `invalid_args`, `confirmation_required`,
  `json_unsupported`).
- **Versioning.** `v` is one number for the whole CLI. New fields and new error codes are compatible (a shell ignores
  what it does not know; an unknown code is treated as `failed`); removing or renaming a field or changing a type or
  a meaning bumps `v`.
- **No localized text.** Data are codes, ids, numbers, paths and RFC 3339 timestamps with the local offset; flavors are
  folder names. The process language is forced to English under `--json`, which makes the one free-text field,
  `error.detail`, English for logs (except `update-addons`, which runs in the user's language so the history it stores
  stays readable in the tray). It is diagnostic: never shown, never parsed.
- **Error codes** are derived from the error, not from its text: the core raises `errors::Coded` (a code, params and the
  text for `Display`, so the text output is unchanged) and `errors::classify` reads it, with I/O errors as `io` and the
  rest as `failed`. The list is in `docs/cli-json.md`.
- **Nothing interactive.** `restore --json` needs `--yes` and never reads stdin.
- **Progress.** Version 1 gives the final object only. Progress can be added as NDJSON lines (`"type":"progress"`
  before the final line) behind a flag without a new `v`; that is why `type` exists.
- **Commands without JSON:** `tray`, `watch`, `quit`, `ui-snapshot` (long-running or windowed), `self-update` and
  `schedule` (they belong to the program itself); they answer `json_unsupported`.

## Consequences

- `docs/cli-json.md` is the reference and has an example per command; the in-source tests assert the same shapes.
- A change of a field is a change of the public interface: it needs a changelog line and, when breaking, a new `v`.
- The macro findings are shaped for JSON in the program (`src/cli_json.rs`), not by serializing the core's types: the
  core's `Finding` is already stored on disk (`macro-report.json`) in its own shape and carries text of the interface
  language.
- Config get/set follows in its own task.
