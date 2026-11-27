# The JSON interface of the command line

`wow-tools --json <command> ...` prints **exactly one line** of JSON on stdout and nothing else, errors included (the one exception is `watch`, which
streams lines: see "Watch stream"). It is
the interface for another program (a second shell such as an Overwolf or Electron app, see
`docs/adr/0002-core-crate-and-shells.md`, the contract is `docs/adr/0003-cli-json-contract.md`). Without `--json` the
commands print text as before; nothing about the text changed.

## Calling it

- Run the exe with `--json` anywhere on the command line (`wow-tools --json status`, `wow-tools backup --json`).
  Pass `--config <file>` to use another config.
- Read all of stdout, parse it as one JSON value. **No JSON on stdout means a crash** (a panic prints to stderr and
  exits 101).
- stderr carries the log lines (English under `--json`; the config's language for `update-addons` and `watch`). Ignore them or keep them for diagnostics; never parse them.
- The process language is forced to English under `--json`; the language setting of the config is not used, except by
  `update-addons` and `watch`, which write the addon history the tray shows: their `error.detail` and stderr are in the
  config's language.
- Nothing is interactive and stdin is never read (except `watch`, where its end means stop). `restore` needs `--yes` (see below).
- Several processes may run at once; two that would change the same game folder exclude each other (error `busy`).
  A shell does not take that lock itself: it calls the CLI.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | success (`"type":"result"`) |
| 1 | the command failed (`"type":"error"`) |
| 2 | wrong use: `invalid_args`, `confirmation_required`, `json_unsupported` |

## The envelope

```json
{"v":1,"type":"result","command":"backup","data":{}}
{"v":1,"type":"error","command":"backup","error":{"code":"busy","params":{},"detail":"..."}}
```

- `v` is the version of this contract, the same for every command. `type` is `result` or `error` (later also
  `progress`, see below). `command` is the command name (`profile list`, `copy-char`, ...), `null` when the command line
  did not parse.
- `error.code` is stable and listed below; `error.params` are the values that go with it. `error.detail` is the error
  chain as English text (the config's language for `update-addons` and `watch`): **diagnostic only, never show it to
  the user and never parse it**.
- An error may also carry `data` beside `error`: a run that did part of its work (`backup`, `macros`, `copy-char`, `clean`, `update-addons`).

### Versioning

Adding a field or an error code is **not** a breaking change: ignore fields and codes you do not know (an unknown code
is handled like `failed`). Removing or renaming a field, changing its type or meaning bumps `v`.

### No text in the data

Everything in `data` is a code, an id, a number, a path or a timestamp. There is no text in the interface language: flavors
are folder names (`_retail_`; translate their titles yourself), enums are lowercase snake_case strings. Timestamps are
RFC 3339 with the local offset (`2026-11-27T10:00:00+03:00`); snapshot ids stay the local `YYYYmmdd-HHMMSS[-label]` strings.
Sizes are bytes. Paths are strings as Windows writes them.

### Progress

The one-shot commands print the final object only. `watch` streams `progress` lines (see "Watch stream").

## Errors

| `code` | `params` | When |
| --- | --- | --- |
| `busy` | `flavor`, `pid`, `operation` (the holder; `null` when unknown) | another process (the tray, `watch`, another CLI) is changing that game folder |
| `game_running` | `game`, `flavor` (`null` when it concerns every flavor) | the game client runs and the command would be overwritten on exit |
| `not_found` | `what` (`snapshot`, `character`, `profile`, `flavor`, `config_key`), `key` (what was asked for) | the lookup found nothing |
| `invalid_args` | `kind` (clap's error kind, snake_case; `invalid_value` also has `key` and `reason`: `type`, `range` or `format`) | the command line is wrong, or `config set` refused the value; `detail` says why |
| `confirmation_required` | none | `restore --json` without `--yes` |
| `no_wow` | none | no WoW install found (or the configured folder holds none) |
| `config_invalid` | `path` | the config is not valid TOML |
| `config_exists` | `path` | `init` over an existing config without `--force` |
| `io` | `kind` (`std::io::ErrorKind`, snake_case), `os_code` | a file operation failed |
| `network` | `host` | an HTTPS request failed |
| `not_run` | none | an addon manager could not run its update (it is open with auto-update off); nothing was checked |
| `partial` | none | a run did part of its work; what it did is in `data` |
| `json_unsupported` | `command` | the command has no JSON form |
| `failed` | none | anything else; read `detail` |

New specific codes may be split off `failed` later.

## Commands

Examples are real output with the paths and times anonymized. `flavor` is a folder name, `kind` is `wtf` or `addons`.
Commands without JSON: `tray`, `quit`, `ui-snapshot`, `self-update` and `schedule` all answer
`json_unsupported` (exit 2): they open windows or belong to the program itself. `watch` is a stream, below.

### `status`

```json
{"v":1,"type":"result","command":"status","data":{"app":{"version":"0.8.0"},"config":{"path":"C:\\Users\\me\\AppData\\Roaming\\WoW Tools\\config\\config.toml","exists":true},"backups":{"dir":"D:\\wow-backups","keep":30},"wow":{"root":"C:\\Games\\World of Warcraft","source":"detected"},"flavors":[{"name":"_retail_","version":"12.1.5.70961","dir":"C:\\Games\\World of Warcraft\\_retail_","selected":true,"characters":2,"series":{"wtf":{"count":3,"last":"2026-11-27T13:14:39+04:00"},"addons":{"count":0,"last":null}}}],"games":[],"addon_apps":[{"app":"curseforge","chosen":true},{"app":"wowup_cf","chosen":false}],"addon_updates":{"enabled":false,"app":"curseforge","last_run":"2026-11-27T12:52:48+04:00"}}}
```

`wow.source` is `config`, `detected` or `battlenet` (flavors came from Battle.net only) or `none`; `wow.error` (an error
object `{code,params,detail}`) replaces the flavors when the configured folder is bad. `flavors[].selected` is whether the
config selects it. `games` are the running game processes (`name`, `dir` or `null`). `addon_apps[].app` is
`curseforge`, `wowup`, `wowup_cf` or `wago`.

### `init`

`wow-tools --json init [--wow-path DIR] [--backup-dir DIR] [--force]`

```json
{"v":1,"type":"result","command":"init","data":{"config_path":"C:\\Users\\me\\AppData\\Roaming\\WoW Tools\\config\\config.toml","config":{"wow_path":"C:\\Games\\World of Warcraft","backup_dir":"D:\\wow-backups","keep":30}}}
```

`config` is the whole written config (here shortened); its names are those of `config.toml`. An existing config is the
error `config_exists`.

### `backup`

```json
{"v":1,"type":"result","command":"backup","data":{"created":[{"flavor":"_retail_","kind":"wtf","id":"20261127-131439","size":594,"created":"2026-11-27T13:14:39+04:00"}],"unchanged":[{"flavor":"_retail_","kind":"addons"}],"skipped_game":null,"failures":[]}}
```

`skipped_game` is the name of the running game that held a flavor back, else `null`. When something failed the answer is
an error `partial` (exit 1) with the same object in `data`; each `failures[]` item is
`{flavor, kind (null when the whole flavor, e.g. busy), error:{code,params,detail}}`.

### `list`

```json
{"v":1,"type":"result","command":"list","data":{"series":[{"flavor":"_retail_","kind":"wtf","snapshots":[{"id":"20261127-131439","label":null,"created":"2026-11-27T13:14:39+04:00","size":594,"path":"D:\\wow-backups\\_retail_\\wtf\\20261127-131439.zip"}]}]}}
```

Newest first. `label` is the part of the id after the time (`pre-restore`, `pre-cleanup`, ...) or `null`.

### `restore`

`wow-tools --json restore [ID|DATE|latest] [--kind K] [--flavor F] --yes`. Without `--yes` the answer is
`confirmation_required` (exit 2) and nothing is changed; stdin is never read.

```json
{"v":1,"type":"result","command":"restore","data":{"flavor":"_retail_","plan":[{"kind":"wtf","snapshot":"20261127-131439"}],"applied":true}}
```

The current state is saved first as a `pre-restore` snapshot, as in text mode.

### `chars`

```json
{"v":1,"type":"result","command":"chars","data":{"flavors":[{"flavor":"_retail_","characters":[{"id":"ACC/Realm/Name","account":"ACC","realm":"Realm","name":"Name","path":"C:\\Games\\World of Warcraft\\_retail_\\WTF\\Account\\ACC\\Realm\\Name"}]}]}}
```

`id` is `ACCOUNT/Realm/Name`, the form `copy-char --from/--to` and `profile save/apply` take.

### `macros`

`wow-tools --json macros [--flavor F] [--update-names]`

```json
{"v":1,"type":"result","command":"macros","data":{"checked":1,"missing":[],"levels":{"broken":1,"warn":0,"info":0},"findings":[{"level":"broken","flavor":"_retail_","scope":{"kind":"character","account":"ACC","realm":"Realm","character":"Name"},"macro":"Fire","line":1,"problem":{"type":"deprecated","func":"GetItemCount","replacement":"C_Item.GetItemCount","patch":"12.1.5","url":"https://warcraft.wiki.gg/wiki/Patch_12.1.5/API_changes"},"fixed":{"variants":[{"lang":"enUS","text":"/run print(C_Item.GetItemCount(1))","chars":34,"over":false,"rest":0,"marks":[{"start":11,"end":30}],"check":false}]}}]}}
```

- `levels` counts the findings of each level (`broken`: does not work, `warn`: may not work, `info`). `missing` are the
  flavors with macros whose name tables are not downloaded yet. `names_updated` (true/false) appears with
  `--update-names`; if the download failed the answer is an error with this object in `data`. The download's progress
  lines go to stderr.
- `scope.kind` is `account` (an account-wide macro; `realm` and `character` are `null`) or `character`. `line` is the
  line of the macro, `0` for a whole-macro finding (`too_long`).
- `problem.type` and its fields: `deprecated` (`func`, `replacement`, `patch`, `url`), `too_long` (`chars`, `max`),
  `brackets`, `secret` (`func`), `unknown_name` (`name`), `wrong_language` (`name`, `lang`, `fix`, `how`: `firm`,
  `maybe`, `client`, `realm`), `alias` (`alias`, `command`, `locales`, `firm`, `bad` and `soft`: lists of
  `{character: "Name-Realm", locale, guessed}`; `guessed`: the realm's language was taken from its name).
- `fixed` is the whole macro corrected, only on the first finding of a macro (`null` on the others and when nothing could
  be fixed). `variants[]`: `lang` (client language(s), `deDE` or `deDE/ruRU`), `text`, `chars`, `over` (longer than the
  game takes: do not offer `text`), `rest` (findings left for the user), `marks` (byte ranges of `text` that were
  replaced), `check` (a replaced function should be tried in the game).

### `copy-char`

`wow-tools --json copy-char --from ACC/Realm/Name --to ACC/Realm/Other [--parts a,b] [--dry-run]`

```json
{"v":1,"type":"result","command":"copy-char","data":{"dry_run":true,"source":"ACC/Realm/Name","targets":[{"character":"ACC/Realm/Other","files":["layout-local.txt"]}]}}
```

`files` are the files copied (or, with `dry_run`, that would be), relative to the character folder. With several `--to` a failure on one target is an error with the targets done before it
in `data`.

### `update-addons`

```json
{"v":1,"type":"result","command":"update-addons","data":{"ran":true,"entries":[{"time":"2026-11-27T13:20:00+04:00","app":"ElvUI","ok":true,"manual":true,"skipped":false,"updated":[{"name":"ElvUI","kind":"addon","from":"15.26","to":"16.00","flavor":"_retail_"}],"failed":[],"outdated":[],"by_hand":[],"error":null},{"time":"2026-11-27T13:20:00+04:00","app":"CurseForge","ok":true,"manual":true,"skipped":false,"updated":[{"name":"BigWigs","kind":"addon","from":"BigWigs-v400.zip","to":"BigWigs-v401.zip","flavor":"_retail_"}],"failed":[],"outdated":["Plater"],"by_hand":["WeakAuras"],"error":null}]}}
```

`entries` are the rows this run wrote to the update history, newest first: one for the addon manager and one for ElvUI
when it updated or failed (`[]` and `ran:false` if none). They carry no text: the summary of a run is stored for the
tray in the language of the user. What the run did is data, from what the manager itself knows:

| key | meaning |
| --- | --- |
| `updated` | addons that got a new version: `name`, `kind` (`addon`, `weakauras`, `ui_packs`; `other` for a kind written by a newer version), `from`, `to` (the manager's own version or file strings, `null` when it has none), `flavor` (`_retail_`, ... or `null` when unknown) |
| `failed` | updates the manager tried and did not finish: `name`, `from`, `to`, `flavor`, `reason` (`no_token`: WowUp got no Wago token; `unknown`, also for a reason written by a newer version); filled also when the row is `ok` and other addons were updated |
| `outdated` | names of addons still behind after the run |
| `by_hand` | names of addons the user updates by hand in the manager; left alone (CurseForge) |
| `error` | `{code, params}` as in the error table when the row failed or was skipped (`skipped:true` has `not_run`), else `null` |

CurseForge names a file by its display name, else its file name, else its id (it has no version field); WowUp gives its `installedVersion`; ElvUI gives the
installed and the site's version per flavor (`from` equal to `to` = reinstalled clean); the Wago App cannot name addons,
so it gives at most two items with `kind` `weakauras` / `ui_packs`, `name` the folder it wrote (`WagoAppCompanion`,
`WagoUI_Storage`) and no versions. The five keys are all `null` for a row that has no details (rows are written with them;
`null` means "unknown", `[]` "none"). The error `partial`
(exit 1, the rows in `data`) means a part failed while another did not (ElvUI failed, the manager did not); when the
manager failed the answer is its error with the rows in `data`. Only the refusal while a game runs (`game_running`) comes
without `data`; other errors before the update (no addon app, a busy folder) carry the history row they wrote. This command starts the chosen addon manager. The history is written in the interface language from the config
(or Windows), not in English, so `error.detail` of this command is in that language too.

### `clean`

Without items, what could be tidied up (read-only):

```json
{"v":1,"type":"result","command":"clean","data":{"items":[{"key":"orphans","files":41,"bytes":9800000,"names":["XPerl","Recount"]},{"key":"backups","files":0,"bytes":0,"names":[]}],"big":[{"name":"Details","bytes":34000000}]}}
```

`key` is `orphans`, `backups`, `logs`, `screenshots` or `cache` (all five are listed). `names` are addon names (orphans).
`big` are the biggest addon settings files. With items (`clean orphans cache`) they are tidied up; an unknown item is
`invalid_args` (kind `invalid_value`, `detail` = the item) before anything is touched:

```json
{"v":1,"type":"result","command":"clean","data":{"results":[{"key":"orphans","files":41,"bytes":9800000,"failed":0,"skipped_flavors":[]}]}}
```

`files` and `bytes` are what was removed or moved, `failed` the files that could not be, `skipped_flavors` the flavors
left alone because their game runs. If an item fails, the answer is its error with the items done before it in `data`
(`results`); when every flavor's game runs the error is `game_running`.

### `outdated`

```json
{"v":1,"type":"result","command":"outdated","data":{"flavors":[{"name":"_retail_","version":"12.1.5.70961","judged":true,"addons":["Auctionator","BugSack"]}]}}
```

`judged` is false (and `addons` empty) when the game version is not known.

### `profile list`

```json
{"v":1,"type":"result","command":"profile list","data":{"profiles":[{"id":"_retail_/20261127-131442 Name - Realm.zip","number":1,"flavor":"_retail_","account":"ACC","realm":"Realm","name":"Name","created":"2026-11-27T13:14:42+04:00","files":28,"addons":["ElvUI","Details"]}]}}
```

Newest first. `id` is stable (`<flavor>/<zip file name>`); `number` shifts when a profile is deleted, so keep the `id`.

### `profile save`

`wow-tools --json profile save ACC/Realm/Name [--flavor F]`

```json
{"v":1,"type":"result","command":"profile save","data":{"profile":{"id":"_retail_/20261127-131442 Name - Realm.zip","number":1,"flavor":"_retail_","account":"ACC","realm":"Realm","name":"Name","created":"2026-11-27T13:14:42+04:00","files":28,"addons":[]},"unchanged":false}}
```

`unchanged` is true when an earlier profile of that character holds the same (`profile` is that one).

### `profile apply`

`wow-tools --json profile apply <id or number> --to ACC/Realm/Other`

```json
{"v":1,"type":"result","command":"profile apply","data":{"profile":{"id":"_retail_/20261127-131442 Name - Realm.zip","number":null,"flavor":"_retail_","account":"ACC","realm":"Realm","name":"Name","created":"2026-11-27T13:14:42+04:00","files":28,"addons":[]},"character":{"id":"ACC/Realm/Other","account":"ACC","realm":"Realm","name":"Other","path":"C:\\Games\\World of Warcraft\\_retail_\\WTF\\Account\\ACC\\Realm\\Other"}}}
```

The game of that flavor must be closed (`game_running`); a `pre-profile` snapshot of WTF is taken first. A profile is
given by its `id` or its number (both work in text mode too).

### `profile delete`

```json
{"v":1,"type":"result","command":"profile delete","data":{"deleted":"_retail_/20261127-131442 Name - Realm.zip"}}
```

### `config get`

`wow-tools --json config get [key]`

```json
{"v":1,"type":"result","command":"config get","data":{"path":"C:\\Users\\Name\\AppData\\Roaming\\WoW Tools\\config\\config.toml","exists":true,"config":{"wow_path":null,"flavors":[],"keep":30,"update_app":null},"defaults":{"wow_path":null,"flavors":[],"keep":30,"update_app":null}}}
{"v":1,"type":"result","command":"config get","data":{"key":"keep","value":30,"default":30}}
```

(`config` and `defaults` hold every setting; four are shown.) The keys are the names in `config.toml`; a setting that is
not set is `null`. An unknown key is `not_found` with `what` `config_key`. `exists` false means the file is not there
yet and the values are the defaults.

### `config set` and `config unset`

`wow-tools --json config set keep 10`, `wow-tools --json config unset update_app`

```json
{"v":1,"type":"result","command":"config set","data":{"key":"keep","value":10,"changed":true}}
{"v":1,"type":"error","command":"config set","error":{"code":"invalid_args","params":{"kind":"invalid_value","key":"keep","reason":"range"},"detail":"keep: the value is out of range"}}
```

The value argument is read as JSON; if that is not JSON it is a plain string. So `10`, `true`, `null`,
`'["mon","fri"]'`, `'{"_retail_":"D:\\Shots"}'` are JSON and `game_exit`, `10:00`, `D:\Shots` are strings. A number
for a string setting (`config set backup_time 1000`) is refused as `type`: write it with quotes (`'"1000"'`).
The whole config is checked before it is written: a wrong type or unknown enum name is `reason` `type`, as is
a list with a tab name the app does not know (`hidden_tabs`, `panel_tabs`); `keep` (1-10000), `backup_every_days`
(1-365) and `update_every` (1-525600 minutes) outside their bounds are `range`; `backup_time` / `update_time` not
`HH:MM`, `flavors` not like `_retail_`, and an empty or relative path in `backup_dir`, `wow_path`, `screenshots_dir`
or `screenshots_dirs` (existence is not checked) are `format`; the file is untouched then. `unset` sets the default (`null` for `wow_path`,
`update_app`, `language`, `screenshots_dir`). `changed` is false when the value was already that.

Only the file is written: the tray reads it again on its next round. The settings window keeps its own copy and writes it
back on its next click, so change settings while it is closed. Comments in a hand-edited `config.toml` are lost (the
file is written whole, as by the settings window), and one key is set per call.

### Errors, examples

```json
{"v":1,"type":"error","command":"backup","error":{"code":"partial","params":{},"detail":"some snapshots were not created"},"data":{"created":[],"unchanged":[],"skipped_game":null,"failures":[{"flavor":"_retail_","kind":null,"error":{"code":"busy","params":{"flavor":"_retail_","pid":4120,"operation":"restore"},"detail":"another WoW Tools is already working on Retail: 4120 restore — try again later"}}]}}
{"v":1,"type":"error","command":"restore","error":{"code":"confirmation_required","params":{},"detail":"restore with --json needs --yes"}}
{"v":1,"type":"error","command":"list","error":{"code":"not_found","params":{"what":"flavor","key":"_nope_"},"detail":"version “_nope_” not found, there are: _retail_"}}
{"v":1,"type":"error","command":null,"error":{"code":"invalid_args","params":{"kind":"unknown_argument"},"detail":"error: unexpected argument '--bogus' found\n..."}}
{"v":1,"type":"error","command":"tray","error":{"code":"json_unsupported","params":{"command":"tray"},"detail":"the command tray has no --json output"}}
```

## Watch stream

`wow-tools --json watch [--interval N]` runs the loop of the tray (scheduled snapshots, after the game closes, patches,
automatic addon updates) and prints **one JSON object per line** (NDJSON) until it is stopped. The rules are in
`docs/adr/0004-watch-json-stream.md`.

- Every line has `"v":1` and `"type"`: `event` or `progress`. Lines are flushed one by one; stderr keeps the log.
  Always read stderr or send it to null (or use `--log-to-file`): a stderr pipe nobody reads fills up and then blocks
  the loop, and backups stop with no sign.
- **Stop it by closing its stdin** (or by exiting: the OS closes it). A job in progress finishes first, then the last line is
  `stopped` and the exit code is 0. Whatever is written to stdin is ignored, so a process started with a closed stdin
  stops after its first round. Killing it is safe too.
- Before the loop starts, a failure (unreadable config) is the usual `error` object of a one-shot command (`command:"watch"`, exit 1).
  Failures of the loop itself are `error` events.
- There is no request channel. To back up or update addons now, run `backup --json` / `update-addons --json`; the folder
  lock (`busy`) keeps them apart from the loop. The loop does not report those runs.
- Not in the stream: WoW Tools self-update (a shell updates itself) and the tray's own states.
- A shell ignores unknown events and fields.

| `event` | `data` |
| --- | --- |
| `started` | `pid`, `version`, `interval_minutes` (`null` without `--interval`): the first line |
| `busy` | `work` (`backup`, `addons`), `manual` (`null`): a job begins; the state is busy until its result event |
| `backup` | the data of `backup` (`created`, `unchanged`, `skipped_game`, `failures`) and `manual`; `failures` may be non-empty |
| `addons` | `manual`, `ok`, and the data of `update-addons` (`ran`, `entries[]`: the history rows this run wrote), `error` when it failed |
| `game` | `state` (`started`, `closed`), `game` (exe name), `flavors[]` (folders whose client runs; empty when its path is unknown). Fires when "any client runs" flips |
| `patch` | `flavor`, `from`, `to`, `saved[]` (snapshot ids taken before the first start), `outdated[]` (addon names) |
| `error` | `source` (`backup`, `addons`), `error` (`code`, `params`, `detail`): a run that failed as a whole, with nothing else to report |
| `stopped` | `reason` (`stdin_closed`): the last line |

`progress` lines come while a snapshot is written (also the ones before an addon update or a patch), at most one per percent:
`data` has `flavor`, `kind` (`wtf`, `addons`), `done` and `total` (bytes), `percent`. They end at the next event line: `backup` for a backup, `addons` (or `error` with source `addons`) for the
`pre-update` snapshot of an addon update, `patch` for a `pre-patch` snapshot, which has no `busy` line before it.

```json
{"v":1,"type":"event","event":"started","time":"2026-11-27T14:39:50+04:00","data":{"pid":31720,"version":"0.8.0","interval_minutes":1}}
{"v":1,"type":"event","event":"busy","time":"2026-11-27T14:39:50+04:00","data":{"work":"backup","manual":null}}
{"v":1,"type":"progress","time":"2026-11-27T14:39:50+04:00","data":{"flavor":"_retail_","kind":"wtf","done":0,"total":7,"percent":0}}
{"v":1,"type":"event","event":"backup","time":"2026-11-27T14:39:50+04:00","data":{"created":[{"flavor":"_retail_","kind":"wtf","id":"20261127-143950","size":193,"created":"2026-11-27T14:39:50+04:00"}],"unchanged":[],"skipped_game":null,"failures":[],"manual":false}}
{"v":1,"type":"event","event":"game","time":"2026-11-27T14:41:02+04:00","data":{"state":"started","game":"Wow.exe","flavors":["_retail_"]}}
{"v":1,"type":"event","event":"stopped","time":"2026-11-27T14:42:10+04:00","data":{"reason":"stdin_closed"}}
```
