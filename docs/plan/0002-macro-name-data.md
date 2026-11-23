# Plan 0002 Macro name data

## Approach
A `names` part inside `macros.rs` (or `macro_names.rs` if it passes ~400 lines): fetch, validate, cache, lookup.
Lookups are `HashMap<String, Vec<u32>>` per (product, locale), loaded lazily from the cache.

## Steps
1. Product mapping: helper from flavor folder to product code using `wow::KNOWN`.
2. `fetch(product, locale)`: latest build via the builds API; if `data\names\<product>-<locale>.meta` has the same
   build, return; else download both CSVs with `selfupdate::http::get` (check it can handle ~17 MB; add a size cap),
   parse `ID,Name_lang`, write the compact cache + meta (build, row count), via a temp file and rename.
3. Validate: compare with the enUS table by id; if over ~90% of rows are identical for a non-enUS locale, reject.
4. Name extraction `command_names(line) -> Vec<String>` (pure, tested).
5. `check_names(macro, locale, tables)` producing findings for 0001's `Finding` (new kinds `WrongLanguage`,
   `UnknownName`); other-locale lookup by id yields the fix text.
6. CLI `--update-names`; progress lines via the logger; locale keys; ignored live test; tests with tiny fixture
   tables.
7. README Privacy + CHANGELOG.

## Files likely touched
`src/macros.rs` (or new `src/macro_names.rs`), `src/main.rs`, `locales/*.toml`, `README*.md`, `CHANGELOG.md`.

## Tradeoffs
- Cache only names, not full CSV: tens of MB less on disk.
- Lowercase exact match over fuzzy: fuzzy is out; a typo shows as "unknown", which is what the owner needs.
- Ship data in repo: rejected (size, redistribution, staleness).
- Classic flavors use their own products (`wow_classic_era` etc.); same code, different product.
