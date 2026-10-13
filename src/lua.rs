//! Just enough of WoW's SavedVariables Lua to find tables and change them in place, keeping the
//! rest of a file byte for byte. The game writes `Name = value` at the top, one `["key"] = value,`
//! per line, array items as `value, -- [n]`, strings in double quotes with backslash escapes.

use std::ops::Range;

/// One `[key] = value,` (or keyless `value,`) of a table.
#[derive(Debug)]
pub struct Entry {
    /// String keys decoded; other keys (`[1]`) as written.
    pub key: Option<String>,
    pub value: Range<usize>,
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    loop {
        while i < s.len() && s[i].is_ascii_whitespace() {
            i += 1;
        }
        if !s[i..].starts_with(b"--") {
            return i;
        }
        while i < s.len() && s[i] != b'\n' {
            i += 1;
        }
    }
}

/// `s[i]` is the opening quote; returns the index after the closing one.
fn skip_string(s: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < s.len() {
        match s[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

fn skip_value(s: &[u8], i: usize) -> Option<usize> {
    match *s.get(i)? {
        b'"' => skip_string(s, i),
        b'{' => Some(table_end(s, i)? + 1),
        _ => {
            let mut j = i;
            while j < s.len() && !matches!(s[j], b',' | b'}' | b'\r' | b'\n') {
                j += 1;
            }
            (j > i).then_some(j)
        }
    }
}

/// Parses the entry at `i`; returns it and the index after its comma.
fn entry(s: &[u8], mut i: usize) -> Option<(Entry, usize)> {
    let mut key = None;
    if s[i] == b'[' {
        let k = skip_ws(s, i + 1);
        let k_end = if s.get(k) == Some(&b'"') {
            skip_string(s, k)?
        } else {
            k + s[k..].iter().position(|&c| c == b']')?
        };
        key = Some(if s[k] == b'"' {
            decode(&s[k..k_end])
        } else {
            String::from_utf8_lossy(&s[k..k_end]).trim().to_string()
        });
        i = skip_ws(s, k_end);
        if s.get(i) != Some(&b']') {
            return None;
        }
        i = skip_ws(s, i + 1);
        if s.get(i) != Some(&b'=') {
            return None;
        }
        i = skip_ws(s, i + 1);
    }
    let end = skip_value(s, i)?;
    let mut j = end;
    while j < s.len() && matches!(s[j], b' ' | b'\t') {
        j += 1;
    }
    if s.get(j) == Some(&b',') {
        j += 1;
    }
    Some((Entry { key, value: i..end }, j))
}

/// The entries of the table whose `{` is at `open`, and the index of its `}`.
fn walk(s: &[u8], open: usize) -> Option<(Vec<Entry>, usize)> {
    let mut out = Vec::new();
    let mut i = open + 1;
    loop {
        i = skip_ws(s, i);
        match s.get(i)? {
            b'}' => return Some((out, i)),
            b';' | b',' => i += 1,
            _ => {
                let (e, next) = entry(s, i)?;
                out.push(e);
                i = next;
            }
        }
    }
}

/// Index of the `}` closing the table opened at `open`.
pub fn table_end(s: &[u8], open: usize) -> Option<usize> {
    walk(s, open).map(|(_, end)| end)
}

pub fn entries(s: &str, open: usize) -> Vec<Entry> {
    walk(s.as_bytes(), open).map(|(e, _)| e).unwrap_or_default()
}

/// The top-level `Name = value` assignments of a file: (name, value).
pub fn globals(s: &str) -> Vec<(String, Range<usize>)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        i = skip_ws(b, i);
        if i >= b.len() {
            return out;
        }
        let name_end = i + b[i..].iter().position(|c| !(c.is_ascii_alphanumeric() || *c == b'_')).unwrap_or(b.len() - i);
        let eq = skip_ws(b, name_end);
        if name_end == i || b.get(eq) != Some(&b'=') {
            return out;
        }
        let v = skip_ws(b, eq + 1);
        let Some(end) = skip_value(b, v) else { return out };
        out.push((s[i..name_end].to_string(), v..end));
        i = end;
    }
}

/// The value of `key` in the table opened at `open`, if it is a table: the index of its `{`.
pub fn subtable(s: &str, open: usize, key: &str) -> Option<usize> {
    entries(s, open)
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .map(|e| e.value.start)
        .filter(|&v| s.as_bytes()[v] == b'{')
}

/// The value of `key` in the table opened at `open`, if it is a string.
pub fn string(s: &str, open: usize, key: &str) -> Option<String> {
    let e = entries(s, open).into_iter().find(|e| e.key.as_deref() == Some(key))?;
    (s.as_bytes()[e.value.start] == b'"').then(|| decode(&s.as_bytes()[e.value.clone()]))
}

/// The source text of `key`'s value in the table opened at `open`.
pub fn value_text<'a>(s: &'a str, open: usize, key: &str) -> Option<&'a str> {
    let e = entries(s, open).into_iter().find(|e| e.key.as_deref() == Some(key))?;
    Some(&s[e.value])
}

/// Sets `key` of the table opened at `open` to `value` (Lua source): replaces its value or adds
/// an entry at the end, in the file's own line ending.
pub fn set(s: &mut String, open: usize, key: &str, value: &str) {
    let b = s.as_bytes();
    let (entries, close) = walk(b, open).expect("a table");
    if let Some(e) = entries.iter().find(|e| e.key.as_deref() == Some(key)) {
        s.replace_range(e.value.clone(), value);
        return;
    }
    let nl = if s.contains("\r\n") { "\r\n" } else { "\n" };
    let line_start = b[..close].iter().rposition(|&c| c == b'\n').map_or(0, |p| p + 1);
    // WoW puts the closing brace on a line of its own; a `{}` written by someone else gets one.
    let lead = if b[line_start..close].iter().all(u8::is_ascii_whitespace) { "" } else { nl };
    let at = if lead.is_empty() { line_start } else { close };
    s.insert_str(at, &format!("{lead}[{}] = {value},{nl}", quote(key)));
}

pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A quoted Lua string (with its quotes) to text.
fn decode(q: &[u8]) -> String {
    let inner = &q[1..q.len().saturating_sub(1).max(1)];
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        if inner[i] != b'\\' || i + 1 == inner.len() {
            out.push(inner[i]);
            i += 1;
            continue;
        }
        let c = inner[i + 1];
        i += 2;
        match c {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'0'..=b'9' => {
                let mut n = u32::from(c - b'0');
                for _ in 0..2 {
                    match inner.get(i) {
                        Some(d @ b'0'..=b'9') => {
                            n = n * 10 + u32::from(d - b'0');
                            i += 1;
                        }
                        _ => break,
                    }
                }
                out.push(n as u8);
            }
            c => out.push(c),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\r\nDB = nil\r\nAceDB = {\r\n[\"profileKeys\"] = {\r\n[\"Гронк - Азурегос\"] = \"Default\",\r\n},\r\n[\"profiles\"] = {\r\n[\"Default\"] = {\r\n[\"text\"] = \"a } \\\" { b\",\r\n[\"list\"] = {\r\n\"x\", -- [1]\r\n2, -- [2]\r\n},\r\n},\r\n},\r\n}\r\nOther = {\r\n}\r\n";

    #[test]
    fn reads_tables() {
        let g = globals(FILE);
        let names: Vec<&str> = g.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["DB", "AceDB", "Other"]);
        let db = g[1].1.start;
        let keys = subtable(FILE, db, "profileKeys").unwrap();
        assert_eq!(string(FILE, keys, "Гронк - Азурегос").as_deref(), Some("Default"));
        let profiles = subtable(FILE, db, "profiles").unwrap();
        let default = subtable(FILE, profiles, "Default").unwrap();
        assert_eq!(string(FILE, default, "text").as_deref(), Some("a } \" { b"));
        assert_eq!(entries(FILE, subtable(FILE, default, "list").unwrap()).len(), 2);
        assert!(value_text(FILE, profiles, "Default").unwrap().starts_with("{\r\n[\"text\"]"));
    }

    #[test]
    fn edits_in_place() {
        let mut s = FILE.to_string();
        let db = globals(&s)[1].1.start;
        let keys = subtable(&s, db, "profileKeys").unwrap();
        set(&mut s, keys, "Гронк - Азурегос", "\"Raid\"");
        let keys = subtable(&s, db, "profileKeys").unwrap();
        set(&mut s, keys, "Alt - Realm", "\"Raid\"");
        assert_eq!(
            s,
            FILE.replace(
                "[\"Гронк - Азурегос\"] = \"Default\",\r\n",
                "[\"Гронк - Азурегос\"] = \"Raid\",\r\n[\"Alt - Realm\"] = \"Raid\",\r\n"
            )
        );
        // An empty table, and one written on one line.
        let other = globals(&s)[2].1.start;
        set(&mut s, other, "k", "1");
        assert!(s.ends_with("Other = {\r\n[\"k\"] = 1,\r\n}\r\n"));
        let mut inline = "X = {}".to_string();
        set(&mut inline, 4, "k", "1");
        assert_eq!(inline, "X = {\n[\"k\"] = 1,\n}");
        assert_eq!(quote("a\"b\\"), "\"a\\\"b\\\\\"");
    }
}
