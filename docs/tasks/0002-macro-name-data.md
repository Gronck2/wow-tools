# 0002 Macro check: spell/item name data and language check

## Goal
Download spell and item names per locale on request, cache them, and use them to find macro names that are in the
wrong language for the character's realm, or unknown (typo/removed).

## Why
The most common real breakage is `/cast Fireball` on a Russian realm or a misspelled localized name
(`Благословление защиты`). Names are not in WTF; they need game data.

## Scope
- Depends on 0001 (parser, realm locale, report).
- Source: wago.tools DB2 CSV `https://wago.tools/db2/{Table}/csv?product={product}&locale={xxYY}` for `SpellName`
  and `ItemSearchName`; build from `https://wago.tools/api/builds/{product}/latest`. Flavor folder -> product via
  `wow::KNOWN` product codes. Network through `selfupdate::http::get`.
- Download only on explicit user request (`wow-tools macros --update-names`, later the UI button); never
  automatically. Cache per product + locale + build in `data\names\` (compact: lowercased name -> ids, deflate via
  the existing `zip` dependency or a plain gzip-free text file if small enough). Offline from cache; re-download only
  when the build changed. Locales needed = locales of realms found for the selected flavors, plus enUS for
  suggestions.
- Validation: a wrong locale silently returns English, so reject a file whose names equal enUS for most rows
  (non-enUS requested) and report an error.
- Check: for `/cast /use /castsequence /castrandom /userandom #showtooltip #show` (and `/stopmacro` is not a name
  command, skip): strip `!`, ranks `(Rank 1)`, `item:123`, numbers, conditionals `[...]`, `reset=...`, split
  castsequence/castrandom by comma. Name found in the realm locale -> ok. Found in another downloaded locale -> "is
  <lang>, use '<realm-locale name>'". Not found anywhere -> warning "unknown name (typo or removed)" (may be an
  equipment set, pet or addon thing). Mounts are spells, toys are items. Account-wide macros: soft wording listing
  the realms where it fails. Without cached data the check is skipped with a one-line hint.
- Network use stated in README "Privacy" (5 languages).

## Out of scope
- Anything from the Blizzard API; bundling the name data in the repo/installer (size, redistribution).
- Auto-fix, UI (0003).

## Acceptance
- Without cache: macros command works as in 0001 and says name data is missing.
- With a fixture cache (tests do not touch the network): wrong language, typo and ok cases produce the verdicts above
  for character and account scope.
- `--update-names` downloads once, second run with the same build does nothing; wrong-locale reply is rejected.
- Live download test is `#[ignore]` like `live_github`.
- Gates pass; CHANGELOG; README Privacy.

## Open owner decisions
1. Download size: retail SpellName is ~17 MB and ItemSearchName ~11.6 MB per locale uncompressed. Recommendation:
   accept, cached compactly, only for the needed locales.
2. wago.tools has no ToS page (robots allows `/db2/*/csv`, llms.txt invites tools and asks for reuse of exports);
   recommendation: proceed with on-request download and a descriptive User-Agent; the owner confirms.
3. ADR: consider one for "third-party game data cache and its network use" (flag for the main thread).
