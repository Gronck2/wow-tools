# Plan 0004 Macro check: ready fixed macro text

## Approach
A pure `macro_fix` module computes, per macro and per target language, the corrected text from the same inputs the
check uses (macro, targets, `Names`). It is computed once in `macros::walk`'s `check_file`, stored as
`Option<Arc<MacroFix>>` on each `Finding` of the macro, rendered by one function for the window, clipboard and CLI.
The fix does not consume `Finding`s: it walks the lines itself with the same predicates (alias locales, name tables,
deprecated list), so the two cannot disagree on what is "wrong" because they share helpers, not copies.

## Steps
1. `macro_names.rs`: replace `command_names` with `name_spans(line) -> Vec<Span { start, end, name }>` (byte offsets in
   the original line; `reset=` tokens and `[...]` skipped by offset, `!` and `(Rank N)` kept outside the span, subtext
   rule as in `check`); `command_names` = spans' names (existing test stays). Add `Names::translate_exact` and
   `Names::has_locale`/`cached(lang)`; `Exact::{One, Ambiguous, None}` as in the task (ids from source spell+item
   `by_name`, names from target `by_id`, drop rows equal to the source text, distinct lowercased count).
2. `data/deprecated-api.tsv`: add column 5 `compat` (`drop-in`/empty) and update the header comment; `macros::deprecated()`
   returns 5-element rows (fix `try_into`, the `Kind::Deprecated` destructuring and any test reading rows).
3. `macros.rs`: `Target::client_main: Option<&'static str>` (set from `client_locale` in `walk`; all `Target { .. }`
   literals in tests get the field); `Finding::fixed: Option<Arc<MacroFix>>`; `check_file` calls
   `macro_fix::build` and attaches the `Arc` to each finding of that macro; `macros::report` prints the variants after
   the macro's first finding (`first_of_macro` = no earlier finding with `Arc::ptr_eq`).
4. New `src/macro_fix.rs`:
   - `target_lang(t, account) -> Option<&'static str>` (ru realm -> ruRU; account: `client_main` else realm; character:
     realm language else `client_main`); group targets into `BTreeMap<lang, Vec<label>>`; drop langs not cached.
   - `fix_line(line, lang, retail, names) -> String`: `/run`/`/script` lines: drop-in deprecated replacement by a small
     scanner that skips Lua strings (`"`, `'`, `[[ ]]` kept simple) and `--` comments; other lines: alias -> canonical
     when the alias's locales do not include `lang`; names via `name_spans` + `translate_exact` when the name is not in
     `lang` but is in another locale; lines with unbalanced brackets returned unchanged.
   - `build`: per language join fixed lines with `\n`, skip if equal to the original, merge equal texts (union targets),
     `chars = text.chars().count()`, `over = chars > MAX_LEN`, `rest` = number of this macro's findings the text does not
     remove (re-run `check` + `macro_names::check` on the fixed macro for the first target and count what remains;
     cheap, reuses the existing checks and doubles as a self-test that the fix converges).
   - Do not clear `text` on `over`; the renderer hides it (keeps `chars` for the message).
5. `report.rs`: `Texts` gets `fixed: Vec<VariantView>` only for a macro's first finding; `card_height` adds caption +
   wrapped monospace block (`gfx::text_format` with a fixed-width family, wrap on) + button row; `Card` stores the Copy
   rects per variant; hit-test/hover in the mouse handlers next to `copy_rect`; click -> `tray::act(Action::CopyFix(text))`;
   `tray.rs` handles it like `CopyMacros` and sets the copy-result line. Over-limit variant: mark only.
6. `main.rs`: `macro-report` demo data: one macro with ruRU and deDE variants, one over-limit; `wow-tools macros` prints
   via `macros::report` (no extra change). 
7. Locales x5: `macros.fixed_caption` ("{lang}, for: {targets}"), `macros.fixed_rest`, `macros.fixed_over`
   (`{n}`, `{max}`), `report.copy_fix`, `report.copied_fix` ({name}); German checked in `ui-snapshot --lang de`.
8. Tests (bottom of `macro_fix.rs`, `macro_names.rs`, `macros.rs`) per the task's Acceptance; fixtures anonymized:
   fake ids and invented/real-neutral names in the style of `macro_names::tests::names()`, extended with an id pair
   that translates two ways (ambiguous) and an untranslated row.
9. Docs: READMEs x5 (macro section: "fixed text, copy it into the game"), CHANGELOG, regenerate
   `macro-report-<lang>[-dark].png` via `ui-snapshot`, `docs/arch/overview.md`, CLAUDE.md Layout (`macro_fix.rs`).
10. Gates: fmt, clippy `-D warnings`, tests; `site/` build if images changed.

## Files likely touched
`src/macro_fix.rs` (new), `src/macro_names.rs`, `src/macros.rs`, `src/report.rs`, `src/tray.rs`, `src/main.rs`,
`data/deprecated-api.tsv`, `locales/*.toml`, `README*.md`, `CHANGELOG.md`, `docs/images/macro-report-*.png`,
`docs/arch/overview.md`, `CLAUDE.md`.

## Tradeoffs
- One text per macro (not per finding): fixes interact (two names on one line, alias + names), and the owner pastes the
  whole macro; the card of the first finding carries it so it is not repeated.
- Spans instead of re-assembling a line from names: keeps every unrelated byte (conditionals, spacing, ranks).
- Canonical English commands instead of localized ones: no new data, valid on all clients.
- Deprecated: a data column plus a tiny string/comment-aware scanner rather than a Lua parser; anything not provably
  the same call shape stays a hint (a wrong silent rewrite of a script is worse than no fix).
- Fix is recomputed through the existing checks (`rest`), so a fix that does not converge is visible, not hidden.
- Memory: the name tables are already loaded inside the background scan and dropped after; the fix runs in the same
  place and keeps only strings.
- Variants by language rather than one "best guess" for account macros: the owner decides which client he plays.

## Implementation notes
- Drop-in verification and the owner's answers are recorded in the task's Decisions (16 rows; `UseContainerItem` left out).
- `fix_line` decides per group of targets: a name is translated only when it is wrong for some target of the group
  (`Names::retarget`, same predicate as `macro_names::check`), an alias only when some target lacks its language.
- `deprecated()` pads a short row to five columns, so a row without the trailing tab still parses.
