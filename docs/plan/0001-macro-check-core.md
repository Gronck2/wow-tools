# Plan 0001 Macro check core

## Approach
One new module `macros.rs`, pure functions over strings plus a thin scan over the WTF tree, in the style of
`maintenance::scan`. Data tables live in `data/*.tsv`, embedded with `include_str!` and parsed once (`OnceLock`).

## Steps
1. `data/realms.tsv`: generate from the LibRealmInfo copy with a one-off script (kept in `tools/` or described in the
   file header, not run at build). Columns: name, english name, locale, region, id. Header carries the zlib notice.
2. `data/deprecated-api.tsv` and `data/slash-aliases.tsv` with source URL comments (warcraft.wiki.gg API change
   pages; aliases from localized GlobalStrings `SLASH_*`, otherwise a hand table with a comment).
3. `macros.rs`:
   - `parse(text) -> Vec<Macro { id, name, icon, body }>`.
   - `Scope::Account | Character { realm, name }`; `scan(ctx, flavors) -> Vec<Found>` walking
     `WTF\Account\*\macros-cache.txt` and `*\*\*\macros-cache.txt`.
   - `realm_locale(flavor_dir, acc, realm) -> Option<Locale>`: name match, else realm id via the `Roster` code in
     `profiles.rs` (make the needed function `pub(crate)`; no copy).
   - `Finding { scope, macro_name, line, kind, fix }`; `kind` an enum, text through `tr!`.
   - `check(macro, flavor_is_retail, locales) -> Vec<Finding>` running checks 1-5 from the task.
   - `report(findings) -> String` (reused later by the UI "copy").
4. `main.rs`: `Macros { --flavor }` subcommand, hidden-free, with `cli.macros.*` help keys.
5. Locale keys in all five files; run the locale tests.
6. Tests inline; CHANGELOG; READMEs.

## Files likely touched
`src/macros.rs` (new), `src/main.rs`, `src/profiles.rs` (visibility only), `data/*.tsv` (new), `locales/*.toml`,
`CHANGELOG.md`, `README*.md`, `docs/arch/overview.md` (layout line for `macros.rs`).

## Tradeoffs
- Static embedded tables over runtime download for realms/API: small, offline, tests are deterministic; stale after
  ~2021 realms, hence "unknown" instead of guessing.
- Whole-word regex-free matching (manual scan) avoids a new dependency.
- Blizzard Game Data API for realms rejected (ToS: no redistribution).
