//! Edit Mode layouts in WTF, for profiles: the source character's layouts are added to the
//! target's instead of replacing them.
//!
//! `edit-mode-cache-account.txt` (account folder): a version, a count and that many account
//! settings, then the account layouts. `edit-mode-cache-character.txt`: the layout of each of six
//! specialization slots, then the character's layouts. A layout is `<byte length> <name> <n>` and
//! `n` frames of ten space-separated tokens (no token has a space; the name may). A slot holds
//! the Lua index into presets + account layouts + character layouts: 1 and 2 are the Modern and
//! Classic presets, 3 the first account layout; 0 is a slot no layout was chosen for, -1 one the
//! character has no specialization for. (Checked on the real files: account #2 has 4 layouts and
//! slots of 6 with no character layouts; a character with 2 of its own uses 8.)

/// Layouts of one type a character or an account may have (`EditModeMaxLayoutsPerType`).
pub const MAX_LAYOUTS: usize = 5;
const SLOTS: usize = 6;
const FRAME_TOKENS: usize = 10;
/// The slot value of the first account layout: 1 and 2 are the presets.
const FIRST_SAVED: i64 = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub name: Vec<u8>,
    /// What follows the name: the frame count and the frames.
    body: Vec<u8>,
}

impl Layout {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(format!("{} ", self.name.len()).as_bytes());
        out.extend_from_slice(&self.name);
        out.push(b' ');
        out.extend_from_slice(&self.body);
    }
}

/// A parsed file: the text before the layouts, the layouts, what ends the file (a NUL).
#[derive(Debug, PartialEq)]
struct File {
    head: Vec<u8>,
    layouts: Vec<Layout>,
    tail: Vec<u8>,
}

impl File {
    fn write(&self) -> Vec<u8> {
        let mut out = self.head.clone();
        for l in &self.layouts {
            out.push(b' ');
            l.write(&mut out);
        }
        out.extend_from_slice(&self.tail);
        out
    }

    fn slots(&self) -> Vec<i64> {
        self.head
            .split(|&b| b == b' ')
            .filter_map(|t| std::str::from_utf8(t).ok()?.parse().ok())
            .collect()
    }
}

struct Cursor<'a> {
    d: &'a [u8],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn end(&self) -> bool {
        self.d[self.i..].iter().all(|b| matches!(b, 0 | b'\r' | b'\n'))
    }

    /// One token and the space after it; the token ends at a space, a NUL or the end.
    fn token(&mut self) -> Option<&'a [u8]> {
        if self.i >= self.d.len() || self.d[self.i] == 0 {
            return None;
        }
        let start = self.i;
        while self.d.get(self.i).is_some_and(|&b| b != b' ' && b != 0) {
            self.i += 1;
        }
        let t = &self.d[start..self.i];
        if self.d.get(self.i) == Some(&b' ') {
            self.i += 1;
        }
        Some(t)
    }

    fn num(&mut self) -> Option<i64> {
        std::str::from_utf8(self.token()?).ok()?.parse().ok()
    }

    /// The end of the last token read (before the space after it).
    fn token_end(&self) -> usize {
        if self.i > 0 && self.d[self.i - 1] == b' ' { self.i - 1 } else { self.i }
    }

    fn layouts(&mut self) -> Option<Vec<Layout>> {
        let mut out = Vec::new();
        while !self.end() {
            let len = usize::try_from(self.num()?).ok()?;
            let name = self.d.get(self.i..self.i + len)?.to_vec();
            self.i += len;
            if self.d.get(self.i) != Some(&b' ') {
                return None;
            }
            self.i += 1;
            let start = self.i;
            let frames = usize::try_from(self.num()?).ok()?;
            for _ in 0..frames * FRAME_TOKENS {
                self.token()?;
            }
            out.push(Layout {
                name,
                body: self.d[start..self.token_end()].to_vec(),
            });
        }
        Some(out)
    }
}

fn parse(d: &[u8], head_tokens: impl FnOnce(&mut Cursor) -> Option<()>) -> Option<File> {
    let mut c = Cursor { d, i: 0 };
    head_tokens(&mut c)?;
    let head = d[..c.token_end()].to_vec();
    let layouts = c.layouts()?;
    let tail = d[c.i.min(d.len())..].to_vec();
    if !layouts.is_empty() && tail.first() == Some(&b' ') {
        return None;
    }
    Some(File { head, layouts, tail })
}

fn parse_character(d: &[u8]) -> Option<File> {
    parse(d, |c| (0..SLOTS).try_for_each(|_| c.num().map(drop)))
}

fn parse_account(d: &[u8]) -> Option<File> {
    parse(d, |c| {
        c.num()?;
        let n = c.num()?;
        (0..n).try_for_each(|_| c.num().map(drop))
    })
}

/// The account layouts of an `edit-mode-cache-account.txt` (empty when it cannot be read).
pub fn account_layouts(d: &[u8]) -> Option<Vec<Layout>> {
    parse_account(d).map(|f| f.layouts)
}

/// What `merge` did.
#[derive(Debug, PartialEq)]
pub struct Merged {
    /// The target's new `edit-mode-cache-character.txt`.
    pub data: Vec<u8>,
    /// Layouts added to the target.
    pub added: Vec<String>,
    /// Layouts that did not fit (the target has `MAX_LAYOUTS` of its own): those slots stay.
    pub full: Vec<String>,
}

/// The source's layouts put on the target: every slot of the target gets the layout the
/// source uses there (slots the source never used take the one it uses most), the layouts the
/// source uses are added to the target's own unless they are there already (an account
/// layout of the target's account, or one of its character layouts), renamed "<name> (<from>)"
/// when the name is taken. `source_account`: the account layouts of the source's account when
/// the profile was saved; `None` (an older profile from another account) leaves slots that point
/// at the source's layouts as they are. `None` when the source or the target is not in this
/// format.
pub fn merge(source: &[u8], source_account: Option<&[Layout]>, target: Option<&[u8]>, target_account: &[Layout], from: &str) -> Option<Merged> {
    let src = parse_character(source)?;
    let mut tgt = match target {
        Some(t) => parse_character(t)?,
        None => File {
            head: Vec::new(),
            layouts: Vec::new(),
            tail: src.tail.clone(),
        },
    };
    let src_slots = src.slots();
    let tgt_slots = if target.is_some() { tgt.slots() } else { vec![0; SLOTS] };
    let used = |v: i64| src_slots.iter().filter(|&&x| x == v).count();
    let most = src_slots.iter().copied().filter(|&v| v >= 1).max_by_key(|&v| used(v));
    let resolve = |v: i64| -> Option<&Layout> {
        let accounts = source_account?;
        let i = usize::try_from(v - FIRST_SAVED).ok()?;
        accounts.get(i).or_else(|| src.layouts.get(i - accounts.len()))
    };

    let account_count = target_account.len() as i64;
    let mut merged = Merged {
        data: Vec::new(),
        added: Vec::new(),
        full: Vec::new(),
    };
    let mut slots = Vec::with_capacity(SLOTS);
    for (i, &v) in src_slots.iter().enumerate() {
        let own = tgt_slots.get(i).copied().unwrap_or(0);
        let v = if v == 0 || v == -1 { most.unwrap_or(v) } else { v };
        if v < FIRST_SAVED {
            slots.push(if v < 1 { own } else { v });
            continue;
        }
        let Some(layout) = resolve(v) else {
            slots.push(own);
            continue;
        };
        if let Some(a) = target_account.iter().position(|l| l == layout) {
            slots.push(FIRST_SAVED + a as i64);
            continue;
        }
        let same = |l: &Layout| l.body == layout.body && (l.name == layout.name || l.name == renamed(&layout.name, from));
        if let Some(c) = tgt.layouts.iter().position(same) {
            slots.push(FIRST_SAVED + account_count + c as i64);
            continue;
        }
        let title = String::from_utf8_lossy(&layout.name).into_owned();
        if tgt.layouts.len() >= MAX_LAYOUTS {
            if !merged.full.contains(&title) {
                merged.full.push(title);
            }
            slots.push(own);
            continue;
        }
        let taken = |n: &[u8]| target_account.iter().chain(&tgt.layouts).any(|l| l.name == n);
        let mut name = layout.name.clone();
        if taken(&name) {
            name = renamed(&layout.name, from);
            let base = name.clone();
            for n in 2.. {
                if !taken(&name) {
                    break;
                }
                name = [&base[..], format!(" {n}").as_bytes()].concat();
            }
        }
        merged.added.push(String::from_utf8_lossy(&name).into_owned());
        tgt.layouts.push(Layout {
            name,
            body: layout.body.clone(),
        });
        slots.push(FIRST_SAVED + account_count + tgt.layouts.len() as i64 - 1);
    }
    tgt.head = slots.iter().map(i64::to_string).collect::<Vec<_>>().join(" ").into_bytes();
    merged.data = tgt.write();
    Some(merged)
}

fn renamed(name: &[u8], from: &str) -> Vec<u8> {
    [name, format!(" ({from})").as_bytes()].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(n: usize, x: &str) -> String {
        let one = format!("0 {x} 1 7 7 UIParent 0.0 45.0 -1 ##$$%/");
        std::iter::repeat_n(one, n).collect::<Vec<_>>().join(" ")
    }

    fn layout(name: &str, x: &str) -> String {
        format!("{} {name} 2 {}", name.len(), frames(2, x))
    }

    fn account(layouts: &[String]) -> Vec<Layout> {
        let text = format!("2 3 1 0 1{}{}\0", if layouts.is_empty() { "" } else { " " }, layouts.join(" "));
        let f = parse_account(text.as_bytes()).unwrap();
        assert_eq!(f.write(), text.as_bytes(), "written back as it was");
        f.layouts
    }

    #[test]
    fn parse_and_write_back() {
        for text in [
            "2 2 2 2 0 0\0".to_string(),
            "2 -1 -1 -1 -1 -1\0".to_string(),
            format!("1 8 6 0 0 0 {} {}\0", layout("1223", "1"), layout("Мой бой", "2")),
        ] {
            let f = parse_character(text.as_bytes()).unwrap();
            assert_eq!(f.write(), text.as_bytes());
        }
        let f = parse_character(format!("1 8 6 0 0 0 {}\0", layout("Мой бой", "2")).as_bytes()).unwrap();
        assert_eq!(f.layouts[0].name, "Мой бой".as_bytes(), "a name with a space, length in bytes");
        assert_eq!(f.slots(), [1, 8, 6, 0, 0, 0]);
        assert!(parse_character(b"2 2 2\0").is_none());
        assert!(
            parse_character(format!("2 2 2 2 0 0 4 Main 3 {}\0", frames(2, "1")).as_bytes()).is_none(),
            "frames missing"
        );
        assert!(parse_account(b"LAYOUT").is_none());
    }

    #[test]
    fn same_account() {
        let acc = account(&[layout("Main", "1")]);
        // The source uses the account layout and one of its own; the target has its own.
        let source = format!("3 4 3 0 0 0 {}\0", layout("Heal", "2"));
        let target = format!("1 1 0 0 0 0 {}\0", layout("Mine", "3"));
        let m = merge(source.as_bytes(), Some(&acc), Some(target.as_bytes()), &acc, "Main - R").unwrap();
        assert_eq!(m.added, ["Heal"]);
        assert_eq!(
            String::from_utf8(m.data.clone()).unwrap(),
            format!("3 5 3 3 3 3 {} {}\0", layout("Mine", "3"), layout("Heal", "2"))
        );
        // Again: nothing new.
        let again = merge(source.as_bytes(), Some(&acc), Some(&m.data), &acc, "Main - R").unwrap();
        assert!(again.added.is_empty());
        assert_eq!(again.data, m.data);
    }

    #[test]
    fn another_account() {
        let src_acc = account(&[layout("Main", "1")]);
        let tgt_acc = account(&[layout("Main", "9"), layout("Other", "8")]);
        let source = "3 3 -1 -1 -1 -1\0";
        // The source's account layout comes as a character layout, renamed: "Main" is taken there.
        let m = merge(source.as_bytes(), Some(&src_acc), None, &tgt_acc, "Main - R").unwrap();
        assert_eq!(m.added, ["Main (Main - R)"]);
        assert_eq!(String::from_utf8(m.data).unwrap(), format!("5 5 5 5 5 5 {}\0", layout("Main (Main - R)", "1")));
        // The same layout on the target's account is used as it is.
        let same = account(&[layout("Main", "1")]);
        let m = merge(source.as_bytes(), Some(&src_acc), Some(b"0 0 0 0 0 0\0"), &same, "Main - R").unwrap();
        assert!(m.added.is_empty());
        assert_eq!(m.data, b"3 3 3 3 3 3\0");
        // An older profile without the account layouts: the target's slots stay.
        let m = merge(source.as_bytes(), None, Some(b"1 0 1 0 0 0\0"), &tgt_acc, "Main - R").unwrap();
        assert_eq!(m.data, b"1 0 1 0 0 0\0");
    }

    #[test]
    fn presets_and_full() {
        let acc = account(&[layout("A", "1")]);
        let m = merge(b"2 2 0 0 -1 -1\0", Some(&[]), Some(b"3 3 3 3 3 3\0"), &acc, "X").unwrap();
        assert_eq!(m.data, b"2 2 2 2 2 2\0", "a preset is spread like a layout");
        let m = merge(b"0 0 0 0 -1 -1\0", Some(&[]), Some(b"3 3 3 3 3 3\0"), &acc, "X").unwrap();
        assert_eq!(m.data, b"3 3 3 3 3 3\0", "nothing chosen: the target's stay");
        let own: Vec<String> = (0..MAX_LAYOUTS).map(|i| layout(&format!("L{i}"), "5")).collect();
        let target = format!("3 3 3 3 3 3 {}\0", own.join(" "));
        let source = format!("3 3 3 0 0 0 {}\0", layout("New", "6"));
        let m = merge(source.as_bytes(), Some(&[]), Some(target.as_bytes()), &[], "X").unwrap();
        assert_eq!(m.full, ["New"]);
        assert_eq!(m.data, target.as_bytes(), "no room: the target stays");
    }

    /// Every real Edit Mode file (read-only) parses and writes back byte for byte; prints the
    /// layouts and slots.
    #[test]
    #[ignore]
    fn live_files() {
        let root = crate::wow::detect_root().unwrap();
        for fl in crate::wow::all_flavors(Some(&root)) {
            for acc in std::fs::read_dir(fl.wtf_dir().join("Account")).into_iter().flatten().flatten() {
                let Ok(d) = std::fs::read(acc.path().join("edit-mode-cache-account.txt")) else {
                    continue;
                };
                let f = parse_account(&d).unwrap_or_else(|| panic!("{}", acc.path().display()));
                assert_eq!(f.write(), d);
                let names: Vec<_> = f.layouts.iter().map(|l| String::from_utf8_lossy(&l.name).into_owned()).collect();
                println!("{} {}: {names:?}", fl.name, acc.file_name().to_string_lossy());
                for c in crate::chars::list(&fl.wtf_dir())
                    .iter()
                    .filter(|c| c.account == acc.file_name().to_string_lossy())
                {
                    let Ok(d) = std::fs::read(c.dir.join("edit-mode-cache-character.txt")) else {
                        continue;
                    };
                    let f = parse_character(&d).unwrap_or_else(|| panic!("{}", c.dir.display()));
                    assert_eq!(f.write(), d);
                    let own: Vec<_> = f.layouts.iter().map(|l| String::from_utf8_lossy(&l.name).into_owned()).collect();
                    println!("   {} {:?} {own:?}", c.name, f.slots());
                }
            }
        }
    }

    /// Every real character's layouts put on every other one, in memory (read-only): the result
    /// parses, keeps the target's own layouts and points only at layouts that exist.
    #[test]
    #[ignore]
    fn live_merge() {
        let root = crate::wow::detect_root().unwrap();
        let (mut runs, mut added) = (0, 0);
        for fl in crate::wow::all_flavors(Some(&root)) {
            let all: Vec<_> = crate::chars::list(&fl.wtf_dir())
                .into_iter()
                .filter_map(|c| {
                    let d = std::fs::read(c.dir.join("edit-mode-cache-character.txt")).ok()?;
                    let acc = std::fs::read(c.dir.parent()?.parent()?.join("edit-mode-cache-account.txt")).ok()?;
                    Some((c, d, account_layouts(&acc)?))
                })
                .collect();
            for (s, sd, sa) in &all {
                for (t, td, ta) in &all {
                    if s.dir == t.dir {
                        continue;
                    }
                    let m = merge(sd, Some(sa), Some(td), ta, &s.name).unwrap();
                    let f = parse_character(&m.data).unwrap();
                    let before = parse_character(td).unwrap();
                    assert_eq!(f.layouts[..before.layouts.len()], before.layouts[..], "{} -> {}", s.name, t.name);
                    assert!(f.layouts.len() <= MAX_LAYOUTS.max(before.layouts.len()));
                    let last = FIRST_SAVED + (ta.len() + f.layouts.len()) as i64;
                    assert!(f.slots().iter().all(|&v| v >= -1 && v < last), "{} -> {}: {:?}", s.name, t.name, f.slots());
                    runs += 1;
                    added += m.added.len();
                }
            }
        }
        println!("{runs} merges, {added} layouts added");
    }
}
