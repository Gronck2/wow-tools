# 0001 Macro check: parser, realm locale, offline checks, CLI

## Goal
`wow-tools macros [--flavor F]` reads every `macros-cache.txt` (account-wide and per character) of the selected
flavors and prints the macros that are broken by checks that need no downloaded data.

## Why
Macros rot silently: removed API in `/run`, bodies over the length limit, bad brackets, localized slash aliases
(`/закл`) on a realm of another language. The owner wants one report instead of finding out in game.

## Scope
- New `src/macros.rs`: parser, checks, report. Read-only over WTF (never writes to the WoW install).
- Format: `WTF\Account\<ACC>\macros-cache.txt` (account) and `...\<Realm>\<Char>\macros-cache.txt` (character).
  A macro is `VER 3 <16 hex id> "<name>" "<icon>"`, body lines, `END`. Empty files exist. Lines not starting with `/`
  or `#` are not commands (addon data such as HealBot `hbCrashProt_*`, `!dosmarkers`) and are ignored.
- Realm locale: `data/realms.tsv` (generated from LibRealmInfo, `name<TAB>english name<TAB>locale<TAB>region<TAB>id`),
  embedded with `include_str!`; LibRealmInfo's zlib notice kept in the file header and in the README credits.
  Realm folder is matched by name (case-insensitive, also the English name), else by realm id from the account's
  `character-list-order.txt` GUIDs (reuse the `Roster` parsing in `profiles.rs`). Region from `Config.wtf`
  `SET portal`. Realm not in the table = locale "unknown", reported as such, never guessed.
- Checks here (all offline):
  1. Deprecated API in `/run` and `/script` (table in `data/deprecated-api.tsv`: function, replacement, removed in
     patch, source URL). Retail flavors only (`_retail_`, `_ptr_`, `_xptr_`, `_beta_`, `_alpha_`); Classic-family
     flavors are skipped. Whole-word match so `C_Spell.GetSpellInfo` is not flagged as `GetSpellInfo`.
  2. Length: body over 255 characters (Retail). The game counts characters, but the check is conservative:
     measured in UTF-16-free `chars()`; documented in the code. See open decision 1.
  3. Unbalanced `[` `]` on a command line.
  4. Localized slash alias (`/закл`) against the realm locale: valid only when the realm is that locale.
     Alias table `data/slash-aliases.tsv` (command, locale, alias) for `/cast /use /castsequence /castrandom
     /startattack /stopcasting /target /focus /assist /click` etc. as far as the source gives them.
  5. Midnight (12.0) secret-values: a `/run` reading combat state (UnitHealth, aura, cooldown functions) gets a soft
     note only.
- Unknown slash commands, unknown conditionals are NOT reported (addons register commands; conditional list is out
  of scope, see below).
- CLI printing in `main.rs`; strings in `locales/{ru,en,de,fr,es}.toml` (`cli.macros.*`, `macros.*`).
- Tests (inline): parser (empty file, non-command bodies, icon number vs name, CRLF), each check, realm match by name
  and id, character vs account-wide verdict. Anonymized fixtures in `tempfile` WTF trees.

## Out of scope
- Spell and item name checks (task 0002), UI (task 0003).
- Valid-conditionals list, `/click` chaining rule (not detectable reliably).
- Writing or fixing macros.

## Acceptance
- On a fake WTF tree the CLI prints findings grouped by "account" / "<character>-<realm>" with macro name, line,
  problem and suggested fix; exit code 0 regardless of findings.
- Account-wide macros: "may not work for: <char-realm>, ..." wording for alias checks (languages of all realms with
  characters on that account); character macros: firm wording against that character's realm.
- Unknown realm locale is stated, not guessed.
- Gates: fmt, clippy, tests; locale tests (all keys in 5 languages, no Russian in code) pass.
- CHANGELOG entry, README (5 languages) section; `ui-snapshot` not needed here.

## Open owner decisions
1. Length unit: the game's limit is 255 characters, but legacy reports say bytes for non-ASCII. Recommendation: flag
   at over 255 chars as an error and over 255 bytes (but within 255 chars) as a soft "may be cut on some clients"
   warning.
2. Classic-family flavors: skip deprecated-API check entirely (recommended; no verified list) vs a small list.
3. ADR: none needed (embedded data tables are not architectural).

## Decisions (orchestrator, 2026-11-24)
- Length: error over 255 characters, soft warning over 255 bytes.
- Classic flavors: the removed-API check is skipped.
- Realm missing from `realms.tsv` but its folder name is Cyrillic -> ruRU (EU Russian realms are the only Cyrillic
  ones); the finding says the language was taken from the realm name.
- LibRealmInfo (zlib) attribution in a `THIRD-PARTY.md` (or the license files) and in the READMEs.
