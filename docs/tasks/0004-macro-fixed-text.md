# 0004 Macro check: ready fixed macro text

## Goal
For a macro with auto-fixable findings, the report (window card, clipboard, `wow-tools macros`) shows the corrected
text of the WHOLE macro, and the window card has a "Copy" button that copies exactly that text. The owner pastes it
into the game himself.

## Why
0001-0003 say what is wrong and give a one-line hint. Retyping a 255-character macro from a hint, with the right
localized spell names, is the slow part. Nothing is written to WoW files: read-only stays a hard rule.

## Scope
- Depends on 0001-0003 (branches off `task/0003-macro-check-ui`).
- New pure module `src/macro_fix.rs`: `build(macro, retail, targets, firm, names) -> Option<MacroFix>`, no I/O.
  `MacroFix` = variants `{ lang, targets: Vec<String>, text, chars, over }` + `rest` (findings of that macro the
  text does not solve). `Scan` is unchanged except that every `Finding` of a macro carries `fixed: Option<Arc<MacroFix>>`
  (the same `Arc` for all findings of one macro). One fixed text per macro, shown on the macro's FIRST finding in sort
  order (the worst level), not repeated on its other cards.
- What is fixed (everything else: hint only, as now):
  1. Spell/item names in the wrong language (`WrongLanguage`): translated through ids, source-language name -> id(s)
     -> name in the target language. Names with no change needed are left byte for byte.
  2. Localized slash aliases (`Alias`): replaced by the canonical English command (`/zauber` -> `/cast`). English
     commands work on every client (also on ruRU), so no table of localized commands is needed; commands that already
     work are kept.
  3. Removed APIs (`Deprecated`), only for the explicit drop-in list below.
- Not fixed, hint only: `TooLong`, `Brackets` (that line is left as is, other lines of the macro are still fixed),
  `Secret`, `UnknownName`, an ambiguous name (see below), a name whose target language is not cached, deprecated
  functions not on the drop-in list.
- Target language: a character's own macro = its realm's language (a Russian realm = ruRU); account-wide macro = the
  owner's client language (`Config.wtf` `textLocale`/`locale`, new `Target::client_main`), a Russian-realm target
  always ruRU, a target with neither falls back to its realm language. If that gives several languages among the
  account's targets: one variant per language, each with the list of targets it is for and its own Copy button.
  Variants with identical text are merged. A language whose name tables are not cached gives no variant.
- Name lookup (`macro_names::Names`): `translate_exact(src_locale, lowercased name, dst_locale) -> Exact::{One(name),
  Ambiguous, None}`. Ids come from `by_name` of the source (spell and item tables together, as the check does), the
  target names from `by_id` of the target locale; target rows that still carry the source's English/same text
  (untranslated rows) are ignored; more than one distinct remaining name = Ambiguous = no fix for that name, the
  line keeps its text and the card says so; none = no fix.
- Name positions: the rewrite works on byte spans of the name inside the line (`name_spans`, the existing
  `command_names` becomes a view over it), so conditionals `[...]`, the `!` prefix, `reset=...`, comma lists of
  `/castsequence /castrandom /userandom`, `;` clauses, `(Rank N)` / subtext and `#showtooltip` / `#show` keep their
  place and text. `item:ID` and numbers are never names and stay.
- Removed APIs, conservative rule. `data/deprecated-api.tsv` gets a fifth column `compat`: `drop-in` or empty. Only
  `drop-in` rows are replaced, and only an occurrence that is (a) a whole word as `find_word` has it, (b) directly
  followed by `(` (optional spaces), (c) outside a Lua string literal and after no `--`, (d) not preceded by
  `function`, `local` or `=` redefinition forms (`function GetItemCount`). `drop-in` means: same arguments and the
  same return values in the same positions (extra trailing returns are fine); anything that returns a table instead
  of a value list (`GetSpellInfo`, `GetSpellCooldown`, `GetSpellCharges`, `GetContainerItemInfo`, `UnitAura` family,
  `GetActionCooldown` ...) is NOT drop-in whatever the call looks like, so `local n = GetSpellInfo(x)`, `select(...)`
  and multiple assignment are never rewritten. Initial `drop-in` set (the implementer re-checks each row against
  warcraft.wiki.gg and leaves a row empty when unsure): `GetContainerNumSlots`, `GetContainerItemLink`,
  `PickupContainerItem`, `UseContainerItem`, `SplitContainerItem`, `SortBags`, `GetItemCount`, `GetItemCooldown`,
  `IsUsableItem`, `GetItemSpell`, `GetItemIcon`, `IsEquippedItem`, `PickupItem`, `UseItemByName`, `EquipItemByName`,
  `IsUsableSpell`, `GetSpellTexture`. `GetMouseFocus` -> `GetMouseFoci()[1]` and the `C_AddOns` group are not
  replaced unless verified drop-in.
- Length: the fixed text is counted in characters (`chars().count()`, as `check` does). More than 255 = `over`: the
  variant shows a mark "N characters, over 255, shorten it by hand" and NO text and NO Copy button (not offered as
  ready). A macro that was too long and whose fix fits is offered normally.
- Text format: body lines joined with `\n`, untouched lines and addon-data lines as in the cache, nothing added.
- Window (`src/report.rs`): under the card's problem/fix lines, per variant: a caption "Fixed macro (<lang>), for:
  <targets>", the text in a wrapped monospace block with its line breaks, and a "Copy" button right-aligned; the
  button copies exactly the shown text (`Action::CopyFix(Arc<str>)`, the clipboard path of `CopyMacros`) and the
  header's copy-result line says what was copied. Card height, layout, hit-testing, hover and the scroll range
  include the block. A line "Still to fix by hand: N" under a text when `rest` > 0.
- Clipboard report and `wow-tools macros` (`macros::report`): after the first finding of a macro, the same variants as
  an indented block (caption, text lines, over-limit mark). One function renders both.
- `ui-snapshot macro-report` demo data gets a macro with two language variants and one over-limit macro.
- Locales x5 (`macros.fixed_*`, `report.copy_fix`), READMEs x5, CHANGELOG, `docs/images/macro-report-<lang>[-dark].png`,
  `docs/arch/overview.md`, CLAUDE.md layout paragraph for `macro_fix.rs`.

## Out of scope
- Writing anything to WoW files or the game (no apply button); fixing `Secret`, `Brackets`, `TooLong`, unknown names.
- Fuzzy matching of typos; translating `/use item:ID`; localized command words other than the canonical English one.
- Rewriting Lua beyond the drop-in word replacement.

## Acceptance
- Tests (anonymized fixtures, fake name tables like `macro_names::tests::names()`): translation de->en, en->ru, a
  character macro on a ruRU realm, an account-wide macro with two client languages (two variants), an ambiguous name
  (no fix, text unchanged), an untranslated-row case, a removed API drop-in replaced and a table-returning one
  (`local n = GetSpellInfo(1)`) left, a string literal / `C_Spell.` field left, a fix longer than 255 (`over`, no text),
  `Brackets` line left while other lines are fixed, `#showtooltip`, `!`, `[..]`, `reset=`, comma list and `(Rank 1)`
  keep their form, report text equals the window's text, `deprecated()` parses 5 columns.
- `ui-snapshot macro-report` ru/en/de light and dark show the fixed block, its Copy button, and the over-limit mark;
  German has no overflow.
- Copy puts exactly the text on the clipboard (checked through the action's payload in a unit test; never touch the
  real clipboard or WoW files in tests).
- Gates: root; `site/` build if images changed.

## Open owner decisions (recommendation first; none block)
1. Partial fix: a macro that still has an unfixable problem (unknown name, brackets) gets its text with the fixable
   parts fixed and a "Still to fix by hand: N" line. Recommendation: offer it (the fixable part is real work saved);
   alternative: offer only when nothing remains.
2. Aliases become the canonical English command, not the target language's own command word. Recommendation: yes
   (works on all clients, no new table).
3. Over-limit variant shows no text at all. Recommendation: yes, as asked ("don't offer it as ready"); alternative:
   show it dimmed without Copy.
4. Account-wide macro with a client language that has no cached tables: no variant for it, the hint stays.
   Recommendation: yes; the status line already tells to download names.

No ADR needed: a pure transformation over existing data; the only external fact (drop-in list) is data, not an
architectural decision. Security-shaped paths touched: `src/main.rs` (snapshot demo, CLI print) only; no write path.

## Decisions
Owner answers to the open questions (recommended options accepted):
1. Partial fix is offered, with a "Still to fix by hand: N" line.
2. Aliases become the canonical English command.
3. An over-limit variant shows no text and no Copy button, only the mark ("N characters, over 255: shorten it by hand").
4. A client language without cached name tables gives no variant; the hint stays.

Drop-in list (verified 2026-11-24). Sources: warcraft.wiki.gg API change pages (Patch 10.0.2, 11.0.0, 11.0.2,
12.0.0, 12.1.5 `/API_changes`), the pages of the old and new functions, and Blizzard's generated API documentation
(`Blizzard_APIDocumentationGenerated/{Container,Item,Spell}Documentation.lua`, wow-ui-source `live`). For the items
group the 12.1.0 shim file `Deprecated_ItemScript.lua` assigns the old global straight to the `C_Item` function
(`GetItemCount = C_Item.GetItemCount`), and `Deprecated_11_0_0.lua` forwards `GetSpellTexture` / `IsUsableSpell` to
`C_Spell` with the same argument.
- `drop-in` (16): `GetContainerNumSlots`, `GetContainerItemLink`, `PickupContainerItem`, `SplitContainerItem`,
  `SortBags` (same arguments and return, `C_Container`), `GetItemCount`, `GetItemCooldown`, `IsUsableItem`,
  `GetItemSpell`, `GetItemIcon` (-> `C_Item.GetItemIconByID`), `IsEquippedItem`, `PickupItem`, `UseItemByName`,
  `EquipItemByName` (`C_Item`), `IsUsableSpell` (-> `C_Spell.IsSpellUsable`, same `usable, noMana`),
  `GetSpellTexture` (first return the icon; `C_Spell` adds trailing returns).
- Left empty: `UseContainerItem` (old 3rd/4th arguments were `target`, `reagentBankAccessible`; the new 4th is
  `bankType`, so a 4-argument call would change meaning), everything returning a table (`GetContainerItemInfo`,
  `GetSpellInfo`, `GetSpellCooldown`, `GetSpellCharges`, `UnitAura` family, `GetActionCooldown`, ...), `GetMouseFocus`,
  the `C_AddOns` group and the rest of the list (not checked as drop-in).

Deviations from the text above: `rest` ("still to fix") is per variant, not on `MacroFix` (variants for different
languages leave different things); an ambiguous name is not named on the card, it is counted in `rest`; the locale
key of the button is `macros.copy_fix` (there is no `report` section); `ui-snapshot macro-report` renders 820 DIPs
high (the window's 600 would cut the fixed block off); `translate_exact` takes the spell ids first and the item ids
only when no spell has the name, as `Names::translate` does.
