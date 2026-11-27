# 0009 `watch --json`: an event stream for a second shell

## Goal
`wow-tools --json watch [--interval N]` runs the same background loop as `watch` and prints one JSON object per line
(NDJSON) on stdout for everything that happens: backups, game start/exit, patches, addon updates, errors, snapshot
progress. A second shell (the Overwolf app, ADR 0002) keeps it as a child process. Without `--json`, `watch` prints and
behaves exactly as before.

## Why
ADR 0003 gives a shell one-shot commands only; `watch` answered `json_unsupported` and the doc said "a shell that wants
automatic backups schedules `backup` itself". The Overwolf shell will instead run `wow-tools watch --json` for the
whole session (option one of ADR 0003's terms: the core's own loop, a process boundary, no second scheduler to keep in
step with `backup_when`/`backup_days`/`update_*`) and show what the tray shows. `type` was kept in the envelope for
exactly this (`event`, `progress`).

## Decisions (made here)
- **Envelope.** Every line is one JSON object, `"v":1`, newline-terminated, flushed per line:
  - `{"v":1,"type":"event","event":"<name>","time":"<rfc3339 local>","data":{...}}`
  - `{"v":1,"type":"progress","time":"..","data":{"flavor":"_retail_","kind":"wtf","done":123,"total":456,"percent":27}}`
  - A failure before the loop starts (bad config path, no `Ctx`) is the ordinary one-shot error object
    (`type:"error"`, `command:"watch"`) and exit 1; nothing else is printed.
  No localized text anywhere (codes, ids, numbers, paths, timestamps; the process language is English under `--json`;
  `error.detail` stays diagnostic). Versioning rule of ADR 0003 applies: a shell ignores unknown events and fields.
- **Events** (`data` in brackets; reuse the shapes of `docs/cli-json.md`):
  - `started` [`pid`, `version`, `interval_minutes` or null]: first line.
  - `busy` [`work`: `backup|addons|restore|cleanup|profile|names`, `manual`: null]: the loop begins a job. State rule for a
    shell: `busy` -> the job's result event -> idle. (In `watch` only `backup` and `addons` occur.)
  - `backup` [`manual`, `created[]`, `unchanged[]`, `skipped_game`, `failures[]`]: exactly the data of the `backup`
    command (one shared builder). A run with `failures` is still a `backup` event; the shell reads `failures`.
  - `patch` [`flavor`, `from`, `to`, `saved[]` (snapshot ids taken before the first start; empty = the game already ran),
    `outdated[]` (addon names)]: from `Event::Patch`. `Patch` gets a `flavor` field (folder name); its `title` is text and stays out.
  - `game` [`state`: `started|closed`, `game`: exe name, `flavors[]`]: from `Event::Game`. It fires when "any client runs"
    flips, as in the tray; `flavors` are the flavors whose folder that client runs from (`wow::running_in`, resolved when
    the event is built; `[]` when the client's path is unknown; on `closed` the ones of the last `started`). A second
    client starting while one runs gives no event (the loop's rule, unchanged). Per-client events would change the loop
    and `Event::Game`; not now.
  - `addons` [`manual`, `ok`, then the `update-addons` data: `ran`, `entries[]`; `error` object when it failed]: the result of an
    automatic addon update. Entries are the history rows written since the `busy`/`addons` event (same `update_data`
    builder as the one-shot command, so task 0010's updated/failed lists appear here without work). `error.code` is
    `not_run` for a manager left open with auto-update off (see 0010; until then `failed`).
  - `error` [`source`: `backup|addons`, `error:{code,params,detail}`]: a run that failed as a whole (`Event::Backup`/`Event::Addons`
    with `Err`: no WoW found, config unreadable, lock busy before any work, manager failed). `errors::classify` gives the code.
  - `stopped` [`reason`: `stdin_closed`]: last line on a graceful stop.
- **The states the tray shows.** busy = `busy` until the job's event; error = `error` or a `backup` with `failures` (the
  shell derives it; the stream carries events, not a duplicated state machine); a new game = `game`. The tray's
  "newer WoW Tools version" (arrow, balloon) is **not** reported: it lives in the tray (`tray::self_update_loop`), not in core, and
  ADR 0003 puts `self-update` outside JSON ("a shell updates itself"). Recorded; no event, no field.
- **Progress lines: yes.** `snapshot::on_progress` already fires from the writing thread once per percent change
  (and at start/end), so a throttle exists. `Progress.what` is text ("Retail . AddOns"), so `Progress` gets `flavor` and `kind`
  fields next to it (additive; the tray keeps `what`). The callback prints a `progress` line (a single `write` of a locked
  stdout: lines never interleave with the loop's). End of a snapshot has no line of its own: the `backup` event closes it.
  Pre-update/pre-patch snapshots report progress too; the shell treats progress as "something is being written".
  Up to ~101 lines per snapshot.
- **Requests: none.** The shell triggers "backup now" / "update addons now" by running the one-shot `backup --json` /
  `update-addons --json`. The folder lock (`busy`) already serializes them with the loop (a shell sees `busy` and retries), no
  `Request` channel on stdin, no second protocol. The loop learns nothing about those runs (`last_done` is read at start); the next
  scheduled snapshot is then "unchanged" by fingerprint: harmless. Revisit only if a shell needs live restore/cleanup progress.
- **Stop.** The shell closes the child's stdin (or dies: then the OS closes it): `watch --json` reads stdin on a thread only
  to detect EOF; the thread owns the `Sender` of the loop's request channel, drops it at EOF, the loop returns (`Disconnected`),
  a `stopped` line is printed, exit 0. A job in progress finishes first. That also ends an orphaned child after a shell crash
  (Windows does not kill children). Killing the process is safe too: lock files are OS locks released on death, the
  snapshot writer and the addon-manager forced switches already recover on the next run (`curseforge-forced.json`,
  `wowup-forced.json`, `wago-kept`). stdin carries no commands (anything read is ignored). A stdout write error (reader gone)
  exits the process with 0 at once (shortcut: equals a kill, acceptable above). `watch --json` started with a closed stdin stops at once:
  documented.
- **Logs** stay on stderr (English). stdout carries only the lines above. Text `watch` and its `log::info!` banners unchanged.
- **ADR.** This changes ADR 0003's rules ("one object per call", "`watch` has no JSON", "stdin is never read"), and ADRs are
  immutable: **flag for the main thread: a new ADR 0004 "watch --json event stream" (envelope, events, stdin-EOF stop, no
  request channel, self-update excluded)**. Not written by this task's author.

## Scope
- JSON branch for `Cmd::Watch` (no more `json_unsupported` for it); an event builder in `src/cli_json.rs` over `watch::Event`;
  the shared `backup_data(report)` builder (the one-shot `backup` uses it too, output unchanged); `Patch.flavor`;
  `snapshot::Progress.{flavor,kind}`; stdin-EOF stop thread; progress callback.
- `docs/cli-json.md` ("Watch stream" section, the "Commands without JSON" paragraph and the stderr/stdin notes corrected),
  `docs/arch/overview.md` (Shells), `CHANGELOG.md` `[Unreleased]`, `CLAUDE.md` Layout line (`cli_json.rs`), the READMEs'
  JSON paragraph (all five, in step) if it says `watch` has no JSON.
- Tests (see Acceptance). No new locale strings (nothing is localized); `cli.watch.json` help is the global flag already.

## Out of scope
A stdin request channel; per-client game events; self-update or tray-only states; reporting config reload failures as events
(they stay `log::warn` on stderr); heartbeat lines; JSON for `tray`; the updated-addon lists themselves (task 0010); config get/set (0008).

## Acceptance
- Gates (root and core): `cargo fmt --all --check`, `cargo clippy --workspace --release --all-targets --locked -- -D warnings`,
  `cargo test --workspace --release --locked`. `site/` untouched.
- Text `watch` output is unchanged (same `cmd_watch` path without `--json`); `backup --json` data is byte for byte as before
  (existing snapshot test passes unchanged).
- A watch round driven in-process: fake WoW tree + scratch config (`flavors = ["_retail_"]`, `skip_while_running = false`,
  scratch `backup_dir`, update schedule off), the request sender dropped before `watch::run(.., Some(1), ..)` so exactly one
  round runs and returns (`Disconnected`); the events collected through the `emit` closure and turned into lines by the event
  builder yield, after normalizing times/ids/root: `busy{work:backup}`, then `backup{manual:false, created:[wtf (and addons)], ...}`
  (a `game` line is filtered out: it depends on the machine). A second round on an unchanged tree gives `unchanged`.
- Builder unit tests, one per event, from hand-made `Event` values (patch, game started/closed with a resolved flavor,
  addons ok with entries, addons `Err` -> `error` with `not_run`/`failed`, backup `Err` -> `error`, failures inside `backup`),
  and for the progress line from a `Progress` value; each line has `v:1`, `type`, no `\n`, parses as one value.
- No localized text: the same tests run with the interface language set to Russian and produce the same lines.
- stdin-EOF stop: the thread/stop function is tested with an in-memory reader (`Cursor`) that ends immediately: the sender is dropped.
- Never starts a real addon manager, never touches the real WoW install, the tray or the game; the tests read `flavors` from the
  fake root only (Battle.net's `product.db` would add real flavors: the config selects `_retail_` and the test asserts the paths
  of the snapshots lie under the temp dir).
- One manual run (scratch config on a fake tree, stdout piped through a JSON parser one line at a time; stdin closed by hand
  -> `stopped`, exit 0).

## Open questions (owner), with recommendation
1. **Stdin EOF stops `watch --json`** (reverses "stdin is never read" for this one command, no commands are read). Recommend
   **yes**: the only portable graceful stop on Windows and it ends orphans.
2. **ADR 0004** for the stream contract: recommend **yes** (the main thread decides; this task does not write it).
3. Self-update is not in the stream: decided here, recommend keeping (the shell updates itself, ADR 0003); say so if the shell
   should show "a newer WoW Tools exists" for the exe it bundles.

## Decided
1. Yes: stdin EOF stops `watch --json` (a `stopped` line, exit 0).
2. Yes: ADR 0004 written (`docs/adr/0004-watch-json-stream.md`).
3. Yes: self-update stays out of the stream.
