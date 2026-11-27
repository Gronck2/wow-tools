# Plan 0009

One PR. Text `watch` is not edited except where a signature must change. JSON lives in `src/cli_json.rs`. Independent of
0010 (the `addons` event reuses `update_data`, which 0010 enriches); if 0010 lands first the event shows its lists. Touches
`src/main.rs` and `src/cli_json.rs` like 0008 does: expect a trivial merge there.

## Facts read
- `watch::run(config_path, explicit_config, interval_override, &Receiver<Request>, emit: impl FnMut(Event))` loops forever; it
  returns only when the `requests` sender is dropped, noticed in `recv_timeout(POLL)` at the end of a round (so one full round
  always runs first). With `interval_override = Some(m)`, `next = now` at start: the first round is due at once.
- `Event`: `Busy(Work)`, `Backup{manual, result: Result<Report>}`, `Restore`, `Addons{manual, result: Result<String>}` (text only),
  `Game(Option<String>)` (exe name; emitted only when "any client runs" flips), `Patch(Vec<Patch>)` (`title` text, `from`, `to`,
  `saved`, `outdated`; no flavor folder name), `Cleanup/Profile/Names` (tray requests only).
- `Report` already has `snapshots`, `unchanged`, `failures`, `skipped_game` (0007); `cli_json::backup` builds the JSON inline.
  `cli_json::update_addons`/`update_data` read history rows written since a start time.
- `snapshot::on_progress(fn())` is a `OnceLock<fn()>` (one per process, no closure), called by `set_progress` on a percent
  change; `snapshot::progress()` returns `Progress{what, done, total}`. Tray registers `post_state`.
- `cmd_watch` keeps `_tx` alive so the loop never ends; logging goes to stderr on the console branch.
- `wow::running_games() -> Vec<Running{name, dir}>`, `wow::running_in(&games, &fl)`.
- No test exists for `watch` today. The loop reads the real machine: `wow::running_game()`, `updater::last_run()` and
  `maintenance::check_patches` (reads/writes `data\game-versions.json` in the real data dir; a fake flavor has no `version`, so it
  is skipped and nothing is written for it).

## Steps
1. **Core, additive.**
   - `maintenance::Patch`: add `pub flavor: String` (`fl.name`) in `check_patches`; fix the places that build a `Patch` (grep, tests).
   - `snapshot::Progress`: add `flavor: String`, `kind: Kind` where the progress is created (the writing function has `fl`
     and `kind`; grep `Progress {`); `what` stays.
   - A function `wow::flavors_running(games: &[Running], flavors: &[Flavor]) -> Vec<String>` (folder names whose client runs, via
     `running_in` per flavor with the same "unknown path counts for every flavor" rule... except report `[]` for a client with no
     path: write the test for both).
2. **Shared builders in `src/cli_json.rs`.** Extract `backup_data(&Report) -> Value` from `backup()` (the function calls it; output
   unchanged). `update_data(entries)` stays. A `fn failure_value(&anyhow::Error) -> Value` is `error_object` already.
3. **The event builder (pure).** `pub struct Stream { flavors: Vec<Flavor>, last_game_flavors: Vec<String>, busy_since: Option<NaiveDateTime> }`
   and `fn lines(&mut self, event: &watch::Event, now: NaiveDateTime) -> Vec<Value>`:
   - `Busy(w)` -> `busy`; remember `now` for `Work::Addons`.
   - `Backup{manual, Ok(r)}` -> `backup` (via `backup_data` + `manual`); `Err(e)` -> `error{source:"backup"}`.
   - `Addons{manual, result}` -> `addons{manual, ok, ..update_data(history rows with time >= busy_since)}`; `Err` adds `error` object
     (`errors::classify`: `game_running`, `not_run` after 0010, else `failed`/`busy`) and emits `error{source:"addons"}`
     too only when there are no entries (a failed run with a row is reported once, by `addons`).
   - `Game(Some(name))` -> `game{started}` with `flavors_running(running_games(), flavors)`; `Game(None)` -> `closed` with the stored list.
   - `Patch(list)` -> one `patch` line per `Patch`.
   - `Restore/Cleanup/Profile/Names` -> `[]` except the `busy`/`Err` of those (never produced by `watch`; map `Err` to `error` with
     `source` = the work name, no special tests beyond one).
   Every value gets `v`, `type`, `event`, `time` (`ts(now)`), `data`. The history is read through `updater::history()`; to keep the
   function testable take the rows from a `fn() -> Vec<Entry>` parameter (default `updater::history`).
   `progress_line(&Progress, now) -> Value`.
4. **Output.** `fn emit_line(&Value)`: `writeln!` on a locked stdout in one call + `flush`; on error `std::process::exit(0)`
   (shortcut: the reader is gone, same as being killed; comment it). The progress callback is `fn on_progress_json()`:
   reads `snapshot::progress()`, `Some(p)` -> `emit_line(progress_line(..))` (no state needed, throttled by the core).
5. **`pub fn watch(cli_cfg..) -> i32` in `cli_json.rs`**: load `Ctx` (failure -> the normal error envelope, exit 1); `i18n` is English
   already; register `snapshot::on_progress(on_progress_json)`; resolve `flavors` (`ctx.flavors(None)` or empty when it fails: a `no_wow`
   shows up later as an `error` event, not a refusal); create the channel, spawn the stdin thread (`stop_on_eof(std::io::stdin(), tx)`:
   `io::copy(&mut r, &mut io::sink())` then `drop(tx)`; generic over `Read` for the test); print `started`; call `watch::run(.., &rx, |e| for l in stream.lines(&e, now()) { emit_line(&l) })`;
   print `stopped`; return 0.
   Wire in `main.rs`: with `--json` and `Cmd::Watch` call it instead of `cli_json::main`'s refusal (the `Cmd::Watch` arm moves out of the
   `json_unsupported` list in `cli_json::run`; `cli_json::main(cli)` is not used for it because it prints one envelope). Keep the clap
   `watch` doc/help keys; `--interval` works.
6. **Tests** (inline, bottom of `src/cli_json.rs`; core ones in their modules):
   - builder tests per event as listed in the task; assert whole `json!` literals after normalizing `time` -> `<ts>`.
   - the in-process round: tempdir with `wow/_retail_/{WTF/Account/ACC/Realm/Name/x.txt, Interface/AddOns/X/X.toc}`, scratch config file
     (written with `Config { wow_path, backup_dir, flavors: vec!["_retail_"], skip_while_running: false, .. }`; confirm in `config.rs` that
     the default update schedule is off, else set it off explicitly), `let (tx, rx) = channel::<Request>(); drop(tx);`, `watch::run(&cfg_path, Some(&cfg_path), Some(1), &rx, |e| events.push(e))`.
     The loop calls `wow::running_game()` for real: drop `game` lines from the assertion. Snapshot paths are asserted to be under the tempdir.
     Round 2 (unchanged) by a second `run`.
   - `stop_on_eof` with `Cursor::new(b"ignored input")`: returns, the receiver reports `Disconnected`.
   - the classifier gives the same lines under Russian (set `i18n` to `Ru` in one test).
   - a text-`watch` regression is the untouched `cmd_watch`; nothing to add.
7. **Docs.** `docs/cli-json.md`: a "Watch stream" section (invocation, stdin rule, event table with one example line each from the
   in-process test output, progress line, the state rule, stop, exit codes, what is not in the stream: self-update, tray-only states);
   fix the "Commands without JSON" paragraph and line 14/16 ("stdin is never read" -> except `watch`). `docs/arch/overview.md`
   Shells. `CHANGELOG.md` Added line. READMEs only if they claim `watch` has no JSON (grep `json_unsupported`/"watch" in the five).
8. **Verify** (PATH from CLAUDE.md): the gates; a manual run against a scratch config and fake tree, output through
   `uv run python -c` parsing each line, stdin closed by `< NUL` (immediate `stopped`) and by a piped `echo`-less pipe that closes later.
   Read-only against the real install is not needed. Never stop the tray; the exe is run from a scratch `--target-dir` build or the
   test binary only.

## Files likely touched
`src/main.rs` (security-shaped: reviewer attention; only the watch arm), `src/cli_json.rs`, `crates/core/src/{watch,snapshot,maintenance,wow}.rs`
(`watch.rs` only gains tests/none), `docs/cli-json.md`, `docs/arch/overview.md`, `CHANGELOG.md`, `CLAUDE.md`, maybe `README*.md`.
`src/tray.rs` untouched (the `Patch`/`Progress` additions are additive; fix constructors if any).

## Tradeoffs considered
- **A stdin request channel** (`{"cmd":"backup"}`) rejected: a second protocol, validation, and the one-shot commands already do it with
  the lock as arbiter. Cost: the loop is unaware of the shell's own runs (harmless, see the task).
- **No stop protocol, kill only** rejected as the sole way: an orphan survives a shell crash on Windows; stdin EOF is ~10 lines.
- **`Event::Game` per flavor/client** rejected now: changes the loop's emit rule and the tray; the boolean plus `flavors` resolved by
  the builder covers "Retail started".
- **Events as a copy of the tray's state machine** (`state: idle|busy|error`) rejected: derivable, and duplicate state drifts.
- **Progress as part of events** rejected: `type:"progress"` was reserved for it, and the shell can ignore the type wholesale.
- **Making `watch::run` take a "stop after N rounds" parameter** for the test rejected: the dropped sender already gives exactly one round.
- **Serialising core types directly** rejected (0007's reason): the event shapes are the CLI's contract, built in the shell layer.
- **Reporting config reload failures** deferred: `log::warn` on stderr is enough for the first shell.
