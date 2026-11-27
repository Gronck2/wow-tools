# 0004. The `watch --json` event stream

Status: accepted (2026-11-27)

## Context

ADR 0003 gave a shell one-shot commands: one object per call, no JSON for `watch`, stdin never read. The Overwolf shell
needs the automatic loop (schedule, game exit, patches, addon updates) without a second scheduler that would have to
follow `backup_when`, `backup_days` and `update_*`. The loop is the core's own (`watch::run`, also the tray's), so the
shell keeps `wow-tools watch` as a child process for the whole session and shows what the tray shows.

## Decision

This supersedes ADR 0003's "one object per call", "`watch` has no JSON" and "stdin is never read" for `watch` only; every
other command keeps them.

- **Stream envelope.** `wow-tools --json watch [--interval N]` prints NDJSON on stdout, one object per line, flushed per
  line, each with `"v":1` and a `"type"`: `event` (`event`, `time`, `data`) or `progress` (`time`, `data`). The version
  and the unknown-fields rule are ADR 0003's. A failure before the loop (config unreadable) is the ordinary one-shot
  `error` object and exit 1. Logs stay on stderr. No localized text: codes, ids, numbers, paths and timestamps.
- **Events.** `started`, `busy`, `backup`, `game`, `patch`, `addons`, `error`, `stopped`; the shapes are in
  `docs/cli-json.md`. They are built in the shell layer (`cli_json::Stream`) from `watch::Event`, reusing the one-shot
  builders (`backup_data`, `update_data`), never by serializing core types. The stream carries events, not a copy of the
  tray's state machine: busy = from `busy` to the result event; error = an `error` event or a `backup` with `failures`.
- **Progress lines.** While a snapshot is written, `type:"progress"` lines with `flavor`, `kind`, `done`, `total`,
  `percent` come from `snapshot::on_progress`, which already fires once per percent change (the tray's throttle). They
  carry codes, not the tray's text (`Progress.what`); `Progress` gained `flavor` and `kind` for it, `Patch` a `flavor`.
- **Stop = stdin EOF.** A thread reads stdin only to see its end and drops the sender of the loop's request channel; the
  loop returns after the running round, a `stopped` line is printed, exit 0. This is the one portable graceful stop on
  Windows and it ends orphans after a shell crash. A killed process is safe too (OS locks, recoverable forced switches).
  Nothing read from stdin is interpreted. A write error on stdout (reader gone) exits at once with 0.
- **No request channel.** The shell triggers "backup now" or "update addons now" by running the one-shot commands; the
  per-flavor folder lock arbitrates with the loop. The loop does not learn about those runs (the next scheduled snapshot
  is "unchanged" by fingerprint).
- **Self-update stays out.** It lives in the tray, and a shell updates itself (ADR 0003).

## Consequences

- `watch` is the only command with a stream; its text mode and the tray are unchanged (additive fields only).
- Later events or fields are compatible; a changed one bumps `v`.
- Per-client game events, a request channel and heartbeats are not done; revisit if a shell needs live restore progress.
