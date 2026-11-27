# The JSON interface of the command line

`wow-tools --json <command> ...` prints **exactly one line** of JSON on stdout and nothing else, errors included. It is
the interface for another program (a second shell such as an Overwolf or Electron app, see
`docs/adr/0002-core-crate-and-shells.md`, the contract is `docs/adr/0003-cli-json-contract.md`). Without `--json` the
commands print text as before; nothing about the text changed.

## Calling it

- Run the exe with `--json` anywhere on the command line (`wow-tools --json status`, `wow-tools backup --json`).
  Pass `--config <file>` to use another config.
- Read all of stdout, parse it as one JSON value. **No JSON on stdout means a crash** (a panic prints to stderr and
  exits 101).
- stderr carries the log lines (English under `--json`). Ignore them or keep them for diagnostics; never parse them.
- The process language is forced to English under `--json`; the language setting of the config is not used.
- Nothing is interactive and stdin is never read. `restore` needs `--yes` (see below).
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
  chain as English text: **diagnostic only, never show it to the user and never parse it**.
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

Version 1 prints the final object only; an operation takes seconds to a minute. Progress lines
(`{"v":1,"type":"progress",...}` before the final line) can be added later without a new `v`, behind a flag.

## Errors

| `code` | `params` | When |
| --- | --- | --- |
| `busy` | `flavor`, `pid`, `operation` (the holder; `null` when unknown) | another process (the tray, `watch`, another CLI) is changing that game folder |
| `game_running` | `game`, `flavor` (`null` when it concerns every flavor) | the game client runs and the command would be overwritten on exit |
| `not_found` | `what` (`snapshot`, `character`, `profile`, `flavor`), `key` (what was asked for) | the lookup found nothing |
| `invalid_args` | `kind` (clap's error kind, snake_case) | the command line is wrong; `detail` is clap's message |
| `confirmation_required` | none | `restore --json` without `--yes` |
| `no_wow` | none | no WoW install found (or the configured folder holds none) |
| `config_invalid` | `path` | the config is not valid TOML |
| `config_exists` | `path` | `init` over an existing config without `--force` |
| `io` | `kind` (`std::io::ErrorKind`, snake_case), `os_code` | a file operation failed |
| `network` | `host` | an HTTPS request failed |
| `partial` | none | a run did part of its work; what it did is in `data` |
| `json_unsupported` | `command` | the command has no JSON form |
| `failed` | none | anything else; read `detail` |

New specific codes may be split off `failed` later.

## Commands

Examples are real output with the paths and times anonymized. `flavor` is a folder name, `kind` is `wtf` or `addons`.
Commands without JSON: `tray`, `watch`, `quit`, `ui-snapshot`, `self-update` and `schedule` all answer
`json_unsupported` (exit 2): they run for a long time, open windows or belong to the program itself. A shell that wants
automatic backups schedules `backup` itself.

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
{"v":1,"type":"result","command":"update-addons","data":{"ran":true,"entries":[{"time":"2026-11-27T13:20:00+04:00","app":"ElvUI","ok":true,"manual":true,"skipped":false},{"time":"2026-11-27T13:20:00+04:00","app":"CurseForge","ok":true,"manual":true,"skipped":false}]}}
```

`entries` are the rows this run wrote to the update history, newest first: one for the addon manager and one for ElvUI
when it updated or failed (`[]` and `ran:false` if none). They carry no text: the summary of a run is stored for the
tray in the language of the user, and the list of updated addons is not available as data yet. The error `partial`
(exit 1, the rows in `data`) means a part failed while another did not (ElvUI failed, the manager did not); when the
manager failed the answer is its error with the rows in `data`. A refusal (a game runs, no addon app) is an error without
`data`. This command starts the chosen addon manager. The history is written in the interface language from the config
(or Windows), not in English, so `error.detail` of this command is in that language too.

### `clean`

Without items, what could be tidied up (read-only):

```json
{"v":1,"type":"result","command":"clean","data":{"items":[{"key":"orphans","files":41,"bytes":9800000,"names":["XPerl","Recount"]},{"key":"backups","files":0,"bytes":0,"names":[]}],"big":[{"name":"Details","bytes":34000000}]}}
```

`key` is `orphans`, `backups`, `logs`, `screenshots` or `cache` (all five are listed). `names` are addon names (orphans).
`big` are the biggest addon settings files. With items (`clean orphans cache`) they are tidied up:

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

### Errors, examples

```json
{"v":1,"type":"error","command":"backup","error":{"code":"partial","params":{},"detail":"some snapshots were not created"},"data":{"created":[],"unchanged":[],"skipped_game":null,"failures":[{"flavor":"_retail_","kind":null,"error":{"code":"busy","params":{"flavor":"_retail_","pid":4120,"operation":"restore"},"detail":"another WoW Tools is already working on Retail: 4120 restore — try again later"}}]}}
{"v":1,"type":"error","command":"restore","error":{"code":"confirmation_required","params":{},"detail":"restore with --json needs --yes"}}
{"v":1,"type":"error","command":"list","error":{"code":"not_found","params":{"what":"flavor","key":"_nope_"},"detail":"version “_nope_” not found, there are: _retail_"}}
{"v":1,"type":"error","command":null,"error":{"code":"invalid_args","params":{"kind":"unknown_argument"},"detail":"error: unexpected argument '--bogus' found\n..."}}
{"v":1,"type":"error","command":"tray","error":{"code":"json_unsupported","params":{"command":"tray"},"detail":"the command tray has no --json output"}}
```
