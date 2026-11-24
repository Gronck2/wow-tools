//! The fixed text of a macro for the macro report: the whole macro with what the check can correct already corrected,
//! per language of the characters it runs for. Pure (no I/O); the owner pastes the text into the game himself.
//!
//! Corrected: names in the wrong language (through spell/item ids), slash aliases of another language (-> the English
//! command, which every client knows), removed functions that have a drop-in replacement (`macros::drop_in`) and removed
//! functions that now return a table, where the macro uses only the first value (`macros::first_value`, `single_value`).
//! Everything else is left as it is and counted in `Variant::rest`. The pieces that were replaced are recorded as byte
//! ranges of the text (`Variant::marks`) so the window can highlight them.

use crate::macro_names::{self, Names, name_spans};
use crate::macros::{self, Kind, MAX_LEN, Macro, Target, canonical_command, lang_name, lang_names};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::Range;

/// The macro as it should read for the characters of one language (or several with the same text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    /// Client language(s) the text is written for, `deDE` or `deDE/ruRU` when the texts came out the same.
    pub lang: String,
    pub text: String,
    pub chars: usize,
    /// Longer than the game takes: the text is not offered, only the mark.
    pub over: bool,
    /// Findings of the macro this text does not solve (too long excluded: `over` says it).
    pub rest: usize,
    /// Byte ranges of `text` that were replaced (new names, commands, functions).
    pub marks: Vec<Range<usize>>,
    /// A function was replaced by one that is not provably the same call: to be tried in the game.
    pub check: bool,
}

impl Variant {
    pub fn caption(&self) -> String {
        tr!("macros.fixed_caption", lang = lang_names(&self.lang, lang_name))
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

    /// The hint that a replaced function must be tried in the game (only for a text that is shown).
    pub fn check_note(&self) -> Option<&'static str> {
        (self.check && !self.over).then(|| tr!("macros.fixed_check"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        for note in v.check_note().map(str::to_string).into_iter().chain(v.note()) {
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

/// Where a call `(...)` that starts `code` ends (the byte after its `)`) and how many arguments it has; none when the
/// parentheses do not close or a comment cuts the call.
/// shortcut: strings are `"..."`, `'...'` and `[[...]]` (no `[=[`), as in `rewrite_code`.
fn call_span(code: &str) -> Option<(usize, usize)> {
    let (mut depth, mut args, mut empty) = (0usize, 1usize, true);
    let mut chars = code.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' | '\'' => {
                while let Some((_, d)) = chars.next() {
                    if d == '\\' {
                        chars.next();
                    } else if d == c {
                        break;
                    }
                }
            }
            '[' if code[i..].starts_with("[[") => {
                let end = i + code[i..].find("]]")? + 2;
                while chars.peek().is_some_and(|(j, _)| *j < end) {
                    chars.next();
                }
            }
            '-' if code[i..].starts_with("--") => return None,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some((i + 1, if empty { 0 } else { args }));
                }
            }
            ',' if depth == 1 => args += 1,
            _ => {}
        }
        if i > 0 && !c.is_whitespace() {
            empty = false;
        }
    }
    None
}

/// The value of a call is used as one value (so the first value of a function is all the code needs), by what stands
/// before the call and after it. Conservative: an operand of an operator, a condition, the single target of an
/// assignment, a parenthesised call, a non-last element of a list, and the sole argument of `print`/`tostring`. A call
/// that is the last element of a list, a `return`, a table constructor, an index or any other call's last argument
/// passes all its values on and is not one of these.
fn single_value(before: &str, after: &str) -> bool {
    const OPS: [&str; 12] = ["..", "+", "-", "*", "/", "%", "^", "==", "~=", "<", ">", "#"];
    const WORDS: [&str; 7] = ["and", "or", "not", "if", "elseif", "while", "until"];
    // a `(` after one of these groups an expression, it does not call
    const KEYWORDS: [&str; 12] = ["and", "or", "not", "if", "elseif", "while", "until", "return", "then", "do", "else", "in"];
    let (b, a) = (before.trim_end(), after.trim_start());
    let a = if a.starts_with("--") { "" } else { a };
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let first_word = |s: &str| s.chars().take_while(|c| ident(*c)).collect::<String>();
    let last_word = |s: &str| {
        let w: String = s.chars().rev().take_while(|c| ident(*c)).collect();
        w.chars().rev().collect::<String>()
    };
    // indexed, called, a string or a table right after
    if a.starts_with(['.', ':', '[', '(', '{', '"', '\'']) && !a.starts_with("..") {
        return false;
    }
    if a.starts_with(',') || OPS.iter().any(|o| a.starts_with(o)) || ["and", "or", "then", "do"].contains(&first_word(a).as_str()) {
        return true;
    }
    if OPS.iter().any(|o| b.ends_with(o)) || WORDS.contains(&last_word(b).as_str()) {
        return true;
    }
    if let Some(pre) = b.strip_suffix('(') {
        let pre = pre.trim_end();
        let word = last_word(pre);
        let call = pre.ends_with([')', ']', '}', '"', '\'']) || (!word.is_empty() && !KEYWORDS.contains(&word.as_str()));
        // a call takes the value on as its argument; only `print` and `tostring` are known to use one
        return a.starts_with(')') && (!call || ["print", "tostring"].contains(&word.as_str()));
    }
    if let Some(lhs) = b.strip_suffix('=') {
        let run: String = lhs
            .chars()
            .rev()
            .take_while(|c| ident(*c) || c.is_whitespace() || matches!(c, '.' | ',' | '[' | ']' | '"' | '\''))
            .collect();
        return !run.trim().is_empty() && !run.contains(',');
    }
    false
}

/// What a rewritten Lua line is: the new code, the byte ranges that changed and whether a replacement needs a try in game.
struct Rewritten {
    code: String,
    marks: Vec<Range<usize>>,
    check: bool,
}

/// Replaces removed functions that can be replaced in Lua `code`: only a call (`Name(`) that is a whole word outside
/// strings and comments, not a field and not being defined. Drop-ins as they are; a function that now returns a table
/// as `(New(args) or {}).field` when the call has the old argument count and only its first value is used; the
/// one-argument functions only with one argument.
/// shortcut: strings are `"..."`, `'...'` and `[[...]]` (no `[=[`), no Lua parser; upgrade if a real macro needs more.
fn rewrite_code(code: &str) -> Rewritten {
    let mut r = Rewritten {
        code: String::new(),
        marks: Vec::new(),
        check: false,
    };
    let (mut i, mut prev) = (0, String::new());
    // (byte of `code` the closing parenthesis ends at, what follows it), innermost last
    let mut closers: Vec<(usize, String)> = Vec::new();
    let flush = |r: &mut Rewritten, closers: &mut Vec<(usize, String)>, at: usize| {
        while closers.last().is_some_and(|c| c.0 == at) {
            let suffix = closers.pop().map(|c| c.1).unwrap_or_default();
            r.marks.push(r.code.len()..r.code.len() + suffix.len());
            r.code.push_str(&suffix);
        }
    };
    while i < code.len() {
        flush(&mut r, &mut closers, i);
        let rest = &code[i..];
        let c = rest.chars().next().unwrap_or_default();
        if rest.starts_with("--") {
            r.code.push_str(rest);
            return r;
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
            r.code.push_str(&rest[..end]);
            i += end;
            prev.clear();
        } else if c.is_alphabetic() || c == '_' {
            let end = rest.find(|d: char| !(d.is_alphanumeric() || d == '_')).unwrap_or(rest.len());
            let word = &rest[..end];
            let after_word = rest[end..].trim_start();
            let before = code[..i].trim_end();
            let field = before.ends_with(':') || (before.ends_with('.') && !before.ends_with(".."));
            let mut replaced = false;
            if after_word.starts_with('(') && !field && prev != "function" && prev != "local" {
                let call = call_span(after_word).map(|(len, args)| (code.len() - after_word.len() + len, args));
                let put = |r: &mut Rewritten, text: &str| {
                    r.marks.push(r.code.len()..r.code.len() + text.len());
                    r.code.push_str(text);
                };
                if let Some(new) = macros::drop_in(word) {
                    put(&mut r, new);
                    replaced = true;
                } else if let Some(new) = macros::one_arg(word)
                    && call.is_some_and(|(_, args)| args == 1)
                {
                    put(&mut r, new);
                    (replaced, r.check) = (true, true);
                } else if let Some((new, member, args)) = macros::first_value(word)
                    && let Some((close, n)) = call
                    && args.contains(&n)
                    && single_value(&code[..i], &code[close..])
                {
                    put(&mut r, &format!("({new}"));
                    closers.push((close, format!(" or {{}}).{member}")));
                    (replaced, r.check) = (true, true);
                }
            }
            if !replaced {
                r.code.push_str(word);
            }
            prev = word.to_string();
            i += end;
        } else {
            r.code.push(c);
            if !c.is_whitespace() {
                prev.clear();
            }
            i += c.len_utf8();
        }
    }
    flush(&mut r, &mut closers, code.len());
    r
}

/// A body line after the fix: its text, what changed in it and whether a replaced function must be tried.
struct Line {
    text: String,
    marks: Vec<Range<usize>>,
    check: bool,
}

/// Replaces `at` of `line` with `new` and moves the marks that lie after it.
fn splice(line: &mut Line, at: Range<usize>, new: &str) {
    let delta = new.len() as isize - (at.end - at.start) as isize;
    line.text.replace_range(at.clone(), new);
    for m in line.marks.iter_mut().filter(|m| m.start >= at.end) {
        *m = m.start.wrapping_add_signed(delta)..m.end.wrapping_add_signed(delta);
    }
    line.marks.push(at.start..at.start + new.len());
}

/// One body line for the targets `group`, who read the macro in `lang`.
fn fix_line(line: &str, lang: &str, group: &[&Target], retail: bool, names: &Names) -> Line {
    let mut out = Line {
        text: line.to_string(),
        marks: Vec::new(),
        check: false,
    };
    let t = line.trim();
    if !t.starts_with(['/', '#']) {
        return out;
    }
    let lead = line.len() - line.trim_start().len();
    let end = lead + t.find(|c: char| c.is_whitespace() || c == '[').unwrap_or(t.len());
    let cmd = line[lead..end].to_lowercase();
    if matches!(cmd.as_str(), "/run" | "/script") {
        if retail {
            let r = rewrite_code(&line[end..]);
            out.text = format!("{}{}", &line[..end], r.code);
            out.marks = r.marks.into_iter().map(|m| m.start + end..m.end + end).collect();
            out.check = r.check;
        }
        return out;
    }
    // A line with unbalanced brackets is left as it is: where its names are cannot be told.
    if !macros::balanced(t) {
        return out;
    }
    let valid: Vec<Vec<&str>> = group.iter().map(|t| t.valid()).collect();
    for span in name_spans(line).iter().rev() {
        // A `(Rank N)` / `(Subtext)` stays in the old language and would still fail: the name is left (and counted).
        if line[span.end..].trim_start().starts_with('(') {
            continue;
        }
        if let Some((len, new)) = names.retarget(&span.name, lang, &valid)
            && len == span.name.len()
        {
            splice(&mut out, span.start..span.start + len, &new);
        }
    }
    let known = macros::alias_locales(&cmd);
    if !known.is_empty() && valid.iter().any(|v| !v.iter().any(|c| known.contains(c))) {
        splice(&mut out, lead..end, &canonical_command(&cmd));
    }
    out
}

/// The fixed text of `m` for `targets`, or none when nothing in it can be corrected. `firm`: the character's own
/// macro (one target), else the account's.
pub fn build(m: &Macro, retail: bool, targets: &[Target], firm: bool, names: &Names) -> Option<MacroFix> {
    let mut groups: BTreeMap<&str, Vec<&Target>> = BTreeMap::new();
    for t in targets {
        // A language without cached names still gets aliases and drop-ins (`retarget` finds nothing there).
        if let Some(lang) = target_lang(t, firm) {
            groups.entry(lang).or_default().push(t);
        }
    }
    let original = m.body.join("\n");
    // (languages, targets, body lines); the same text for several languages is one variant
    let mut found: Vec<(Vec<&str>, Vec<&Target>, Vec<Line>)> = Vec::new();
    for (lang, group) in groups {
        let body: Vec<Line> = m.body.iter().map(|l| fix_line(l, lang, &group, retail, names)).collect();
        let text = body.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        if text == original {
            continue;
        }
        match found
            .iter_mut()
            .find(|f| f.2.iter().map(|l| l.text.as_str()).eq(body.iter().map(|l| l.text.as_str())))
        {
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
            let text = body.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
            let (mut marks, mut at) = (Vec::new(), 0);
            for l in &body {
                marks.extend(l.marks.iter().map(|m| m.start + at..m.end + at));
                at += l.text.len() + 1;
            }
            marks.sort_by_key(|m| m.start);
            let chars = text.chars().count();
            let own: Vec<Target> = group.iter().map(|t| (*t).clone()).collect();
            let fixed = Macro {
                name: String::new(),
                icon: String::new(),
                body: body.iter().map(|l| l.text.clone()).collect(),
            };
            let rest = macros::check(&fixed, retail, &own, firm)
                .into_iter()
                .chain(macro_names::check(&fixed, &own, firm, names))
                .filter(|(_, k)| !matches!(k, Kind::TooLong { .. }))
                .count()
                + macro_names::marked(&fixed);
            Variant {
                lang: langs.join("/"),
                text,
                chars,
                over: chars > MAX_LEN,
                rest,
                marks,
                check: body.iter().any(|l| l.check),
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
                "/cast [@focus,harm] !Fireball(Rank 1); [mod] Смерть и разложение",
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
        // the same text for two languages is one variant for both
        let both = texts(&["/закл Foo"], true, &[target("A-De", "deDE"), target("B-En", "enUS")], false);
        assert_eq!(both, [("deDE/enUS".to_string(), "/cast Foo".to_string())]);
        // a language whose names are not cached gives no variant for names...
        assert!(texts(&["/cast Fireball"], true, &[target("C-Fr", "frFR")], true).is_empty());
    }

    #[test]
    fn a_language_without_tables_still_gets_aliases_and_drop_ins() {
        let fr = [target("A-Fr", "frFR")];
        let got = texts(&["/run GetItemCount(5)", "/закл Feuerball"], true, &fr, true);
        assert_eq!(
            got,
            [(
                "frFR".into(),
                "/run C_Item.GetItemCount(5)
/cast Feuerball"
                    .into()
            )]
        );
    }

    #[test]
    fn a_rank_or_subtext_in_the_old_language_keeps_the_name_and_is_counted() {
        let ru = [target("A-Ru", "ruRU")];
        assert!(texts(&["/cast Fireball(Rank 1)", "/cast Fireball (Racial)"], true, &ru, true).is_empty());
        let fix = build(&mac(&["/cast Fireball(Rank 1)", "/cast Fireball"]), true, &ru, true, &names()).unwrap();
        assert_eq!(
            fix.variants[0].text,
            "/cast Fireball(Rank 1)
/cast Огненный шар"
        );
        assert_eq!(fix.variants[0].rest, 1);
    }

    #[test]
    fn a_name_that_means_two_things_or_has_an_untranslated_row_is_not_guessed() {
        let ru = [target("A-Ru", "ruRU")];
        // deDE Sturmangriff = Charge: ruRU has only the untranslated English row
        let n = Names::fake(&[
            ("enUS", &[(3, "Charge")], &[]),
            ("ruRU", &[(3, "Charge")], &[]),
            ("deDE", &[(3, "Sturmangriff")], &[]),
        ]);
        assert!(build(&mac(&["/cast Sturmangriff"]), true, &ru, true, &n).is_none());
        // the same text is a spell in deDE and in frFR with different Russian names: no guess
        let n = Names::fake(&[
            ("enUS", &[(1, "A"), (2, "B")], &[]),
            ("ruRU", &[(1, "Один"), (2, "Два")], &[]),
            ("deDE", &[(1, "Wort")], &[]),
            ("frFR", &[(2, "Wort")], &[]),
        ]);
        assert!(build(&mac(&["/cast Wort"]), true, &ru, true, &n).is_none());
        // the second meaning has no Russian row at all: still no guess for the player of that language
        let n = Names::fake(&[
            ("enUS", &[(1, "A"), (2, "B")], &[]),
            ("ruRU", &[(1, "Один")], &[]),
            ("deDE", &[(1, "Wort")], &[]),
            ("frFR", &[(2, "Wort")], &[]),
        ]);
        assert!(build(&mac(&["/cast Wort"]), true, &ru, true, &n).is_none());
        // a spell and an item of one name with different Russian names: no guess
        let n = Names::fake(&[("enUS", &[(1, "Potion")], &[(5, "Potion")]), ("ruRU", &[(1, "Зелье")], &[(5, "Эликсир")])]);
        assert!(build(&mac(&["/use Potion"]), true, &ru, true, &n).is_none());
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
            "/run print(\"GetItemCount(1)\")",
            "/run x = 'GetItemCount(1)' -- GetItemCount(2)",
            "/run print(C_Item.GetItemCount(1), obj:GetItemCount(1))",
            "/run function GetItemCount(x) end",
            "/run local GetItemCount = GetItemCount",
            "/run print(\"n: \"..C_Item.GetItemCount(1))",
        ] {
            assert_eq!(fixed(line), None, "{line}");
        }
        // `..` is concatenation, not a field
        assert_eq!(
            fixed("/run print(\"n: \"..GetItemCount(5))").as_deref(),
            Some("/run print(\"n: \"..C_Item.GetItemCount(5))")
        );
        // GetSpellTexture / IsUsableSpell took (slot, bookType) too: replaced only with one argument
        assert_eq!(fixed("/run x = IsUsableSpell(1)").as_deref(), Some("/run x = C_Spell.IsSpellUsable(1)"));
        assert_eq!(
            fixed("/run x = GetSpellTexture(\"Fireball\")").as_deref(),
            Some("/run x = C_Spell.GetSpellTexture(\"Fireball\")")
        );
        assert_eq!(fixed("/run x = IsUsableSpell(1, \"spell\")"), None);
        assert_eq!(fixed("/run x = GetSpellTexture()"), None);
        // only Retail has removed functions
        assert!(texts(&["/run print(GetItemCount(5))"], false, &en, true).is_empty());
        // partly fixed: the table-returning call stays and is counted
        let fix = build(&mac(&["/run print(GetItemCount(5), GetSpellInfo(1))"]), true, &en, true, &names()).unwrap();
        assert_eq!(fix.variants[0].text, "/run print(C_Item.GetItemCount(5), GetSpellInfo(1))");
        assert_eq!(fix.variants[0].rest, 1);
    }

    /// The fixed `/run` line of `code` for a Retail character, or none.
    fn run(code: &str) -> Option<String> {
        let en = [target("A-En", "enUS")];
        texts(&[&format!("/run {code}")], true, &en, true)
            .into_iter()
            .next()
            .map(|t| t.1.trim_start_matches("/run ").to_string())
    }

    #[test]
    fn a_table_function_gets_its_first_field_where_one_value_is_used() {
        let name = "(C_Spell.GetSpellInfo(x) or {}).name";
        for (code, fixed) in [
            ("print(GetSpellInfo(x))", format!("print({name})")),
            ("tostring(GetSpellInfo(x))", format!("tostring({name})")),
            ("if GetSpellInfo(x) then y() end", format!("if {name} then y() end")),
            ("while GetSpellInfo(x) do end", format!("while {name} do end")),
            ("local a = GetSpellInfo(x)", format!("local a = {name}")),
            ("a = GetSpellInfo(x) print(a)", format!("a = {name} print(a)")),
            ("t.n = GetSpellInfo(x)", format!("t.n = {name}")),
            ("print(GetSpellInfo(x) == \"A\")", format!("print({name} == \"A\")")),
            ("print(\"n: \"..GetSpellInfo(x))", format!("print(\"n: \"..{name})")),
            ("print(not GetSpellInfo(x))", format!("print(not {name})")),
            ("y = (GetSpellInfo(x))", format!("y = ({name})")),
            ("y = GetSpellInfo(x) or \"?\"", format!("y = {name} or \"?\"")),
            ("f(GetSpellInfo(x), 1)", format!("f({name}, 1)")),
            ("local n, m = GetSpellInfo(x), 2", format!("local n, m = {name}, 2")),
            ("return GetSpellInfo(x), 1", format!("return {name}, 1")),
            ("y = GetSpellInfo (x) -- why", "y = (C_Spell.GetSpellInfo (x) or {}).name -- why".to_string()),
            ("y = GetSpellInfo(f(1, 2))", "y = (C_Spell.GetSpellInfo(f(1, 2)) or {}).name".into()),
            ("y = GetSpellInfo(\"A, B\")", "y = (C_Spell.GetSpellInfo(\"A, B\") or {}).name".into()),
        ] {
            assert_eq!(run(code), Some(fixed), "{code}");
        }
        // the other functions: their own field and argument counts
        let cases = [
            ("print(GetSpellCooldown(5))", "print((C_Spell.GetSpellCooldown(5) or {}).startTime)"),
            ("print(GetSpellCharges(5))", "print((C_Spell.GetSpellCharges(5) or {}).currentCharges)"),
            (
                "print(GetContainerItemInfo(0, 1))",
                "print((C_Container.GetContainerItemInfo(0, 1) or {}).iconFileID)",
            ),
            (
                "print(UnitBuff(\"player\", 1))",
                "print((C_UnitAuras.GetBuffDataByIndex(\"player\", 1) or {}).name)",
            ),
            (
                "print(UnitDebuff(\"player\", 1, \"HARMFUL\"))",
                "print((C_UnitAuras.GetDebuffDataByIndex(\"player\", 1, \"HARMFUL\") or {}).name)",
            ),
            (
                "print(UnitAura(\"player\", 1))",
                "print((C_UnitAuras.GetAuraDataByIndex(\"player\", 1) or {}).name)",
            ),
            ("print(GetActionCooldown(1))", "print((C_ActionBar.GetActionCooldown(1) or {}).startTime)"),
            ("print(GetActionCharges(1))", "print((C_ActionBar.GetActionCharges(1) or {}).currentCharges)"),
        ];
        for (code, fixed) in cases {
            assert_eq!(run(code).as_deref(), Some(fixed), "{code}");
        }
        // not the old call: the (index, bookType) form, a missing or an extra argument
        for code in [
            "print(GetSpellInfo(1, \"spell\"))",
            "print(GetSpellInfo())",
            "print(GetContainerItemInfo(0))",
            "print(UnitBuff(\"player\"))",
        ] {
            assert_eq!(run(code), None, "{code}");
        }
    }

    #[test]
    fn several_values_used_means_no_fix() {
        for code in [
            "local n, r, i = GetSpellInfo(x)",
            "n, r = GetSpellInfo(x)",
            "print(1, GetSpellInfo(x))",
            "print(select(3, GetSpellInfo(x)))",
            "local i = select(n, GetSpellInfo(x))",
            "return GetSpellInfo(x)",
            "local t = {GetSpellInfo(x)}",
            "f(GetSpellInfo(x))",
            "f(1, GetSpellInfo(x))",
            "y = GetSpellInfo(x).foo",
            "y = GetSpellInfo(x)[1]",
            "y = t[GetSpellInfo(x)]",
            "y = \"GetSpellInfo(x)\"",
            "y = C_Spell.GetSpellInfo(x)",
            "y = obj:GetSpellInfo(x)",
            "function GetSpellInfo(x) end",
            "y = GetSpellInfo",
        ] {
            assert_eq!(run(code), None, "{code}");
        }
        // left alone and still counted
        let en = [target("A-En", "enUS")];
        let fix = build(
            &mac(&["/run print(GetSpellInfo(1)) local a, b = GetSpellCooldown(1)"]),
            true,
            &en,
            true,
            &names(),
        )
        .unwrap();
        assert_eq!(
            fix.variants[0].text,
            "/run print((C_Spell.GetSpellInfo(1) or {}).name) local a, b = GetSpellCooldown(1)"
        );
        assert_eq!(fix.variants[0].rest, 1);
    }

    #[test]
    fn replaced_pieces_are_recorded_and_a_replaced_function_must_be_tried() {
        let en = [target("A-En", "enUS")];
        let body = [
            "/cast Feuerball",
            "/run print(GetItemCount(5)) print(GetSpellInfo(1))",
            "/run y = GetSpellInfo(7) z = IsUsableSpell(2)",
            "-- Feuerball",
        ];
        let fix = build(&mac(&body), true, &en, true, &names()).unwrap();
        let v = &fix.variants[0];
        let pieces: Vec<&str> = v.marks.iter().map(|m| &v.text[m.clone()]).collect();
        assert_eq!(
            pieces,
            [
                "Fireball",
                "C_Item.GetItemCount",
                "(C_Spell.GetSpellInfo",
                " or {}).name",
                "(C_Spell.GetSpellInfo",
                " or {}).name",
                "C_Spell.IsSpellUsable"
            ]
        );
        assert!(v.check && v.check_note() == Some(tr!("macros.fixed_check")));
        assert!(render(&fix).contains(&format!("      {}\n", tr!("macros.fixed_check"))));
        // plain drop-ins and names need no try
        let fix = build(&mac(&["/cast Feuerball", "/run print(GetItemCount(5))"]), true, &en, true, &names()).unwrap();
        assert!(!fix.variants[0].check && fix.variants[0].check_note().is_none());
    }

    #[test]
    fn marks_stay_on_character_boundaries_after_several_replacements() {
        let ru = [target("A-Ru", "ruRU")];
        let body = [
            "/cast [@focus] Fireball; Death and Decay",
            "/закл Огненный шар",
            "/run print(\"Огонь\", GetItemCount(5))",
        ];
        let fix = build(&mac(&body), true, &ru, true, &names()).unwrap();
        let v = &fix.variants[0];
        assert!(v.marks.windows(2).all(|w| w[0].end <= w[1].start));
        // every mark cuts the text on character boundaries (indexing would panic otherwise)
        let pieces: Vec<&str> = v.marks.iter().map(|m| &v.text[m.clone()]).collect();
        assert_eq!(pieces, ["Огненный шар", "Смерть и разложение", "C_Item.GetItemCount"]);
    }

    #[test]
    fn the_caption_names_the_language_and_no_character() {
        let ru = [target("Secret-Char", "ruRU")];
        let fix = build(&mac(&["/cast Fireball"]), true, &ru, true, &names()).unwrap();
        assert_eq!(fix.variants[0].caption(), tr!("macros.fixed_caption", lang = tr!("game_lang.ruRU")));
        assert!(!fix.variants[0].caption().contains("Secret"));
        let fix = build(&mac(&["/закл Foo"]), true, &[target("A", "deDE"), target("B", "enUS")], false, &names()).unwrap();
        assert_eq!(fix.variants[0].lang, "deDE/enUS");
        assert!(
            fix.variants[0]
                .caption()
                .contains(&format!("{} / {}", tr!("game_lang.deDE"), tr!("game_lang.enUS")))
        );
    }

    #[test]
    fn a_crafted_quality_name_is_left_and_counted_not_unknown() {
        let n = Names::fake(&[
            ("enUS", &[], &[(5, "Potion"), (6, "Potion"), (7, "Potion"), (8, "Toy Boat")]),
            ("ruRU", &[], &[(5, "Зелье"), (6, "Зелье"), (7, "Зелье"), (8, "Лодка")]),
        ]);
        let ru = [target("A-Ru", "ruRU")];
        let go = |body: &[&str]| build(&mac(body), true, &ru, true, &n);
        // a potion with three crafted ranks and a toy: one name for all ids, translated
        let fixed = go(&["/use Potion", "/use [mod] Toy Boat", "/castsequence Potion, Toy Boat"]).unwrap();
        assert_eq!(fixed.variants[0].text, "/use Зелье\n/use [mod] Лодка\n/castsequence Зелье, Лодка");
        // ids, equipment slots and bag slots are no names
        assert!(go(&["/use item:5", "/use 13", "/use 14", "/use 0 1"]).is_none());
        // the quality icon sits in the name: not matched, never unknown, counted for the hand next to a real fix
        let marked = "/use Potion |A:Professions-ChatIcon-Quality-Tier2:17:23|a";
        assert!(macro_names::check(&mac(&[marked]), &ru, true, &n).is_empty());
        let fix = go(&["/use Potion", marked]).unwrap();
        assert_eq!(fix.variants[0].text, format!("/use Зелье\n{marked}"));
        assert_eq!(fix.variants[0].rest, 1);
        assert!(go(&[marked]).is_none());
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
