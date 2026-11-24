//! The fixed text of a macro for the macro report: the whole macro with what the check can correct already corrected,
//! per language of the characters it runs for. Pure (no I/O); the owner pastes the text into the game himself.
//!
//! Corrected: names in the wrong language (through spell/item ids), slash aliases of another language (-> the English
//! command, which every client knows) and removed functions that have a drop-in replacement (`macros::drop_in`).
//! Everything else is left as it is and counted in `Variant::rest`.

use crate::macro_names::{self, Names, name_spans};
use crate::macros::{self, Kind, MAX_LEN, Macro, Target, canonical_command};
use std::collections::BTreeMap;

/// The macro as it should read for the characters of one language (or several with the same text).
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    /// Client language(s) the text is written for, `deDE` or `deDE/ruRU` when the texts came out the same.
    pub lang: String,
    pub targets: Vec<String>,
    pub text: String,
    pub chars: usize,
    /// Longer than the game takes: the text is not offered, only the mark.
    pub over: bool,
    /// Findings of the macro this text does not solve (too long excluded: `over` says it).
    pub rest: usize,
}

impl Variant {
    pub fn caption(&self) -> String {
        tr!(
            "macros.fixed_caption",
            lang = self.lang,
            targets = crate::maintenance::names_text(&self.targets, 6)
        )
    }

    /// The text to show and copy; none when it is over the limit.
    pub fn shown(&self) -> Option<&str> {
        (!self.over).then_some(self.text.as_str())
    }

    /// The line under the text: why it is not offered, or what is left to do by hand.
    pub fn note(&self) -> Option<String> {
        if self.over {
            Some(tr!("macros.fixed_over", n = self.chars, max = MAX_LEN))
        } else {
            (self.rest > 0).then(|| tr!("macros.fixed_rest", n = self.rest))
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MacroFix {
    pub variants: Vec<Variant>,
}

/// The variants as indented lines for the clipboard report and the console.
pub fn render(fix: &MacroFix) -> String {
    let mut out = String::new();
    for v in &fix.variants {
        out.push_str(&format!("      {}\n", v.caption()));
        for line in v.shown().into_iter().flat_map(|t| t.split('\n')) {
            out.push_str(&if line.is_empty() { "\n".into() } else { format!("        {line}\n") });
        }
        if let Some(note) = v.note() {
            out.push_str(&format!("      {note}\n"));
        }
    }
    out
}

/// The language a target should read the macro in: a Russian realm always ruRU; a character's own macro in its
/// realm's language, an account-wide one in the player's client language (the realm's language when there is none).
fn target_lang(t: &Target, firm: bool) -> Option<&'static str> {
    let realm = t.loc.map(|l| l.code);
    if t.ru() {
        Some("ruRU")
    } else if firm {
        realm.or(t.client_main)
    } else {
        t.client_main.or(realm)
    }
}

/// Replaces removed functions that have a drop-in replacement in Lua `code`: only a call (`Name(`) that is a
/// whole word outside strings and comments, not a field and not being defined.
/// shortcut: strings are `"..."`, `'...'` and `[[...]]` (no `[=[`), no Lua parser; upgrade if a real macro needs more.
fn replace_dropin(code: &str) -> String {
    let (mut out, mut i, mut prev) = (String::new(), 0, String::new());
    while i < code.len() {
        let rest = &code[i..];
        let c = rest.chars().next().unwrap_or_default();
        if rest.starts_with("--") {
            out.push_str(rest);
            break;
        }
        let end = if c == '"' || c == '\'' {
            let mut chars = rest.char_indices().skip(1);
            let mut end = rest.len();
            while let Some((j, d)) = chars.next() {
                if d == '\\' {
                    chars.next();
                } else if d == c {
                    end = j + 1;
                    break;
                }
            }
            Some(end)
        } else if rest.starts_with("[[") {
            Some(rest.find("]]").map_or(rest.len(), |p| p + 2))
        } else {
            None
        };
        if let Some(end) = end {
            out.push_str(&rest[..end]);
            i += end;
            prev.clear();
        } else if c.is_alphabetic() || c == '_' {
            let end = rest.find(|d: char| !(d.is_alphanumeric() || d == '_')).unwrap_or(rest.len());
            let word = &rest[..end];
            let call = rest[end..].trim_start().starts_with('(');
            let field = code[..i].trim_end().ends_with(['.', ':']);
            match macros::drop_in(word) {
                Some(new) if call && !field && prev != "function" && prev != "local" => out.push_str(new),
                _ => out.push_str(word),
            }
            prev = word.to_string();
            i += end;
        } else {
            out.push(c);
            if !c.is_whitespace() {
                prev.clear();
            }
            i += c.len_utf8();
        }
    }
    out
}

/// One body line for the targets `group`, who read the macro in `lang`.
fn fix_line(line: &str, lang: &str, group: &[&Target], retail: bool, names: &Names) -> String {
    let t = line.trim();
    if !t.starts_with(['/', '#']) {
        return line.to_string();
    }
    let lead = line.len() - line.trim_start().len();
    let end = lead + t.find(|c: char| c.is_whitespace() || c == '[').unwrap_or(t.len());
    let cmd = line[lead..end].to_lowercase();
    if matches!(cmd.as_str(), "/run" | "/script") {
        return if retail {
            format!("{}{}", &line[..end], replace_dropin(&line[end..]))
        } else {
            line.to_string()
        };
    }
    // A line with unbalanced brackets is left as it is: where its names are cannot be told.
    if !macros::balanced(t) {
        return line.to_string();
    }
    let valid: Vec<Vec<&str>> = group.iter().map(|t| t.valid()).collect();
    let mut out = line.to_string();
    for span in name_spans(line).iter().rev() {
        if let Some((len, new)) = names.retarget(&span.name, lang, &valid) {
            out.replace_range(span.start..span.start + len, &new);
        }
    }
    let known = macros::alias_locales(&cmd);
    if !known.is_empty() && valid.iter().any(|v| !v.iter().any(|c| known.contains(c))) {
        out.replace_range(lead..end, &canonical_command(&cmd));
    }
    out
}

/// The fixed text of `m` for `targets`, or none when nothing in it can be corrected. `firm`: the character's own
/// macro (one target), else the account's.
pub fn build(m: &Macro, retail: bool, targets: &[Target], firm: bool, names: &Names) -> Option<MacroFix> {
    let mut groups: BTreeMap<&str, Vec<&Target>> = BTreeMap::new();
    for t in targets {
        if let Some(lang) = target_lang(t, firm).filter(|l| names.has(l)) {
            groups.entry(lang).or_default().push(t);
        }
    }
    let original = m.body.join("\n");
    // (languages, targets, body lines); the same text for several languages is one variant
    let mut found: Vec<(Vec<&str>, Vec<&Target>, Vec<String>)> = Vec::new();
    for (lang, group) in groups {
        let body: Vec<String> = m.body.iter().map(|l| fix_line(l, lang, &group, retail, names)).collect();
        if body.join("\n") == original {
            continue;
        }
        match found.iter_mut().find(|f| f.2 == body) {
            Some(f) => {
                f.0.push(lang);
                f.1.extend(group);
            }
            None => found.push((vec![lang], group, body)),
        }
    }
    let variants: Vec<Variant> = found
        .into_iter()
        .map(|(langs, group, body)| {
            let text = body.join("\n");
            let chars = text.chars().count();
            let own: Vec<Target> = group.iter().map(|t| (*t).clone()).collect();
            let fixed = Macro {
                name: String::new(),
                icon: String::new(),
                body,
            };
            let rest = macros::check(&fixed, retail, &own, firm)
                .into_iter()
                .chain(macro_names::check(&fixed, &own, firm, names))
                .filter(|(_, k)| !matches!(k, Kind::TooLong { .. }))
                .count();
            Variant {
                lang: langs.join("/"),
                targets: group.iter().map(|t| t.label.clone()).collect(),
                text,
                chars,
                over: chars > MAX_LEN,
                rest,
            }
        })
        .collect();
    (!variants.is_empty()).then_some(MacroFix { variants })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::Loc;

    fn names() -> Names {
        Names::fake(&[
            (
                "enUS",
                &[(1, "Fireball"), (2, "Death and Decay"), (3, "Charge"), (20, "Smite"), (21, "Smite")],
                &[(10, "Healing Potion")],
            ),
            (
                "ruRU",
                &[
                    (1, "Огненный шар"),
                    (2, "Смерть и разложение"),
                    (3, "Charge"),
                    (20, "Кара"),
                    (21, "Священный огонь"),
                ],
                &[(10, "Лечебное зелье")],
            ),
            ("deDE", &[(1, "Feuerball"), (2, "Tod und Verfall")], &[(10, "Heiltrank")]),
        ])
    }

    fn target(label: &str, code: &'static str) -> Target {
        Target {
            label: label.into(),
            loc: Some(Loc { code, guessed: false }),
            client_main: None,
            client: Vec::new(),
        }
    }

    fn mac(body: &[&str]) -> Macro {
        Macro {
            name: "T".into(),
            icon: "1".into(),
            body: body.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// (language, text) of every variant.
    fn texts(body: &[&str], retail: bool, targets: &[Target], firm: bool) -> Vec<(String, String)> {
        build(&mac(body), retail, targets, firm, &names())
            .map(|f| f.variants.into_iter().map(|v| (v.lang, v.text)).collect())
            .unwrap_or_default()
    }

    #[test]
    fn translates_between_languages_and_keeps_the_form_of_the_line() {
        let en = [target("A-En", "enUS")];
        assert_eq!(texts(&["/cast Feuerball"], true, &en, true), [("enUS".into(), "/cast Fireball".into())]);
        // a character's own macro on a ruRU realm; conditionals, `!`, rank, `;`, reset= and comma lists stay where they are
        let ru = [target("A-Ru", "ruRU")];
        let body = [
            "#showtooltip Fireball",
            "/cast [@focus,harm] !Fireball(Rank 1); [mod] Death and Decay",
            "/castsequence reset=5/target Fireball, Death and Decay",
            "/use Healing Potion",
            "/use 13",
            "-- addon data line",
        ];
        let fixed = texts(&body, true, &ru, true);
        assert_eq!(
            fixed[0].1,
            [
                "#showtooltip Огненный шар",
                "/cast [@focus,harm] !Огненный шар(Rank 1); [mod] Смерть и разложение",
                "/castsequence reset=5/target Огненный шар, Смерть и разложение",
                "/use Лечебное зелье",
                "/use 13",
                "-- addon data line"
            ]
            .join("\n")
        );
        // names that work already stay byte for byte
        assert!(texts(&["/cast  [help]  Огненный шар ;Кара"], true, &ru, true).is_empty());
    }

    #[test]
    fn an_account_macro_gets_a_variant_per_client_language() {
        let mut ru = target("A-Ru", "ruRU");
        ru.client_main = Some("deDE");
        let mut de = target("B-De", "deDE");
        de.client_main = Some("deDE");
        let got = texts(&["/cast Fireball"], true, &[ru.clone(), de.clone()], false);
        assert_eq!(
            got,
            [
                ("deDE".to_string(), "/cast Feuerball".to_string()),
                ("ruRU".to_string(), "/cast Огненный шар".to_string())
            ]
        );
        let fix = build(&mac(&["/cast Fireball"]), true, &[ru, de], false, &names()).unwrap();
        assert_eq!(
            (fix.variants[0].targets.clone(), fix.variants[1].targets.clone()),
            (vec!["B-De".to_string()], vec!["A-Ru".to_string()])
        );
        // the same text for two languages is one variant for both
        let both = texts(&["/закл Foo"], true, &[target("A-De", "deDE"), target("B-En", "enUS")], false);
        assert_eq!(both, [("deDE/enUS".to_string(), "/cast Foo".to_string())]);
        // a language whose names are not cached gives no variant
        assert!(texts(&["/cast Fireball"], true, &[target("C-Fr", "frFR")], true).is_empty());
    }

    #[test]
    fn an_ambiguous_or_untranslated_name_is_left_and_counted() {
        let ru = [target("A-Ru", "ruRU")];
        // two ids called Smite have two Russian names: no guess
        assert!(texts(&["/cast Smite"], true, &ru, true).is_empty());
        // an untranslated row is no answer either
        assert!(texts(&["/cast Charge"], true, &ru, true).is_empty());
        let fix = build(&mac(&["/cast Fireball", "/cast Smite"]), true, &ru, true, &names()).unwrap();
        assert_eq!(fix.variants[0].text, "/cast Огненный шар\n/cast Smite");
        assert_eq!(fix.variants[0].rest, 1);
        assert_eq!(fix.variants[0].note(), Some(tr!("macros.fixed_rest", n = 1)));
        // an unbalanced line is left, the others are fixed
        let fix = build(&mac(&["/cast [@focus Fireball", "/cast Fireball"]), true, &ru, true, &names()).unwrap();
        assert_eq!(fix.variants[0].text, "/cast [@focus Fireball\n/cast Огненный шар");
        assert_eq!(fix.variants[0].rest, 1);
        // nothing wrong, nothing offered
        assert!(texts(&["/cast Огненный шар"], true, &ru, true).is_empty());
    }

    #[test]
    fn ids_stay_and_every_name_command_is_fixed() {
        let ru = [target("A-Ru", "ruRU")];
        let body = [
            "#show Fireball",
            "/use item:10",
            "/use [mod] item:10; Healing Potion",
            "/castrandom Fireball, Death and Decay",
            "/userandom Healing Potion",
        ];
        let fixed = &texts(&body, true, &ru, true)[0].1;
        assert_eq!(
            fixed.as_str(),
            "#show Огненный шар\n/use item:10\n/use [mod] item:10; Лечебное зелье\n/castrandom Огненный шар, Смерть и разложение\n/userandom Лечебное зелье"
        );
    }

    #[test]
    fn aliases_become_the_english_command() {
        let en = [target("A-En", "enUS")];
        assert_eq!(texts(&["/закл [@focus] Огненный шар"], true, &en, true)[0].1, "/cast [@focus] Fireball");
        // an alias the client knows stays
        assert!(texts(&["/закл Foo"], true, &[target("A-Ru", "ruRU")], true).is_empty());
    }

    #[test]
    fn removed_functions_are_replaced_only_when_it_is_the_same_call() {
        let en = [target("A-En", "enUS")];
        let fixed = |line: &str| texts(&[line], true, &en, true).into_iter().next().map(|t| t.1);
        assert_eq!(
            fixed("/run print(GetItemCount(5), GetContainerNumSlots (0))").as_deref(),
            Some("/run print(C_Item.GetItemCount(5), C_Container.GetContainerNumSlots (0))")
        );
        // a table instead of values, a string, a comment, a field, a definition, a mention without a call
        for line in [
            "/run local n = GetSpellInfo(1)",
            "/run print(\"GetItemCount(1)\")",
            "/run x = 'GetItemCount(1)' -- GetItemCount(2)",
            "/run print(C_Item.GetItemCount(1), obj:GetItemCount(1))",
            "/run function GetItemCount(x) end",
            "/run local GetItemCount = GetItemCount",
        ] {
            assert_eq!(fixed(line), None, "{line}");
        }
        // only Retail has removed functions
        assert!(texts(&["/run print(GetItemCount(5))"], false, &en, true).is_empty());
        // partly fixed: the table-returning call stays and is counted
        let fix = build(&mac(&["/run print(GetItemCount(5), GetSpellInfo(1))"]), true, &en, true, &names()).unwrap();
        assert_eq!(fix.variants[0].text, "/run print(C_Item.GetItemCount(5), GetSpellInfo(1))");
        assert_eq!(fix.variants[0].rest, 1);
    }

    #[test]
    fn the_length_is_counted_in_characters_and_an_overlong_text_is_not_offered() {
        let long = |name: &str, n: usize| vec![format!("/cast {name}"); n];
        let body = |v: &Vec<String>| mac(&v.iter().map(String::as_str).collect::<Vec<_>>());
        // too long as written, fits once fixed: offered
        let (en, ru) = ([target("A-En", "enUS")], [target("A-Ru", "ruRU")]);
        let v = long("Feuerball", 17);
        let fix = build(&body(&v), true, &en, true, &names()).unwrap();
        assert!(macros::check(&body(&v), true, &en, true).iter().any(|(_, k)| matches!(k, Kind::TooLong { .. })));
        assert!(!fix.variants[0].over && fix.variants[0].chars <= MAX_LEN && fix.variants[0].rest == 0);
        assert_eq!(fix.variants[0].shown(), Some(fix.variants[0].text.as_str()));
        // fits as written, too long once fixed: only the mark
        let fix = build(&body(&long("Fireball", 17)), true, &ru, true, &names()).unwrap();
        let v = &fix.variants[0];
        assert!(v.over && v.chars == v.text.chars().count() && v.chars > MAX_LEN && v.shown().is_none());
        assert_eq!(v.note(), Some(tr!("macros.fixed_over", n = v.chars, max = MAX_LEN)));
        assert!(!render(&fix).contains("/cast"));
    }

    #[test]
    fn the_report_text_is_the_shown_text() {
        let ru = [target("A-Ru", "ruRU")];
        let fix = build(&mac(&["#showtooltip Fireball", "/cast Fireball"]), true, &ru, true, &names()).unwrap();
        let text = render(&fix);
        assert!(text.starts_with(&format!("      {}\n", fix.variants[0].caption())));
        let lines: Vec<&str> = text.lines().skip(1).collect();
        assert_eq!(lines, ["        #showtooltip Огненный шар", "        /cast Огненный шар"]);
    }
}
