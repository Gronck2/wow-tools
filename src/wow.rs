use anyhow::{Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use sysinfo::{ProcessesToUpdate, System};

/// Game client executable names, lowercase. Only a fallback: a client is recognized by the flavor
/// folder it runs from, because the names differ between flavors and change (Forever's beta runs
/// `WowB.exe` from `_classic_beta_`).
const GAME_EXES: &[&str] = &[
    "wow.exe",
    "wow-64.exe",
    "wowt.exe",
    "wowb.exe",
    "wowclassic.exe",
    "wowclassict.exe",
    "wowclassicb.exe",
];

/// Flavor folders in display order, with their titles; unknown ones are titled by name. Battle.net
/// product codes are in each folder's `.flavor.info`. Sources: WowUp, instawow (`PRODUCTS`).
const KNOWN: &[(&str, &str)] = &[
    ("_retail_", "Retail"),                   // wow
    ("_ptr_", "PTR"),                         // wowt
    ("_xptr_", "XPTR"),                       // wowxptr
    ("_beta_", "Beta"),                       // wow_beta
    ("_forever_", "Forever"),                 // not out yet; the name is a guess
    ("_classic_", "Classic"),                 // wow_classic: the current progression Classic
    ("_classic_ptr_", "Classic PTR"),         // wow_classic_ptr
    ("_classic_beta_", "Classic Beta"),       // wow_classic_beta: WoW Forever beta in 2026
    ("_classic_era_", "Classic Era"),         // wow_classic_era: Era, Hardcore, Season of Discovery
    ("_classic_era_ptr_", "Classic Era PTR"), // wow_classic_era_ptr
    ("_classic_era_beta_", "Classic Era Beta"),
    ("_anniversary_", "Anniversary"),     // wow_anniversary: TBC Anniversary realms
    ("_alpha_", "Alpha"),                 // wowdev
    ("_classic_alpha_", "Classic Alpha"), // wowdev2
    ("_event1_", "Event"),                // wowe1
    ("_submission_", "Submission"),       // wowz
];

/// One game flavor: `_retail_`, `_classic_`, `_classic_era_`, ... Usually all of them are in
/// one install folder, but Battle.net can put a flavor into another one.
#[derive(Debug, Clone)]
pub struct Flavor {
    pub name: String,
    pub dir: PathBuf,
    /// Client version from Battle.net ("1.60.1.70009"), when known.
    pub version: Option<String>,
}

impl Flavor {
    pub fn new(name: impl Into<String>, dir: impl Into<PathBuf>) -> Flavor {
        Flavor {
            name: name.into(),
            dir: dir.into(),
            version: None,
        }
    }

    pub fn wtf_dir(&self) -> PathBuf {
        self.dir.join("WTF")
    }

    pub fn title(&self) -> String {
        let title = flavor_title(&self.name);
        // Classic slots (beta, PTR) running a 1.60+ client hold WoW Forever (instawow's version ranges).
        if self.version.as_deref().is_some_and(is_forever_version) && title.starts_with("Classic") {
            return title.replacen("Classic", "Forever", 1);
        }
        title
    }
}

fn is_forever_version(v: &str) -> bool {
    let mut parts = v.split('.').map(|p| p.parse::<u32>().ok());
    matches!((parts.next(), parts.next()), (Some(Some(1)), Some(Some(minor))) if minor >= 60)
}

/// "Retail", "Classic Era", "PTR", ... for the UI.
pub fn flavor_title(name: &str) -> String {
    if let Some((_, title)) = KNOWN.iter().find(|(n, _)| *n == name) {
        return title.to_string();
    }
    name.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| match w {
            "ptr" | "xptr" => w.to_uppercase(),
            _ => {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn order(name: &str) -> (usize, &str) {
    (KNOWN.iter().position(|(n, _)| *n == name).unwrap_or(KNOWN.len()), name)
}

/// `_retail_`, `_classic_era_`, `_dark_realm_2_`, `_vendor10_`, ...
fn is_flavor_name(name: &str) -> bool {
    name.len() > 2 && name.starts_with('_') && name.ends_with('_') && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn is_client_exe(file_name: &str) -> bool {
    let n = file_name.to_lowercase();
    GAME_EXES.contains(&n.as_str()) || (n.starts_with("wow") && n.ends_with(".exe"))
}

/// A flavor folder: Battle.net's `.flavor.info`, settings or addons, or at least a client.
fn looks_like_flavor(dir: &Path) -> bool {
    if dir.join(".flavor.info").is_file() || dir.join("WTF").is_dir() || dir.join("Interface").is_dir() {
        return true;
    }
    fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|e| is_client_exe(&e.file_name().to_string_lossy())))
}

fn sort(list: &mut [Flavor]) {
    list.sort_by(|a, b| order(&a.name).cmp(&order(&b.name)));
}

/// All game flavors found in the install root.
pub fn flavors(root: &Path) -> Vec<Flavor> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<Flavor> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let dir = e.path();
            (is_flavor_name(&name) && dir.is_dir() && looks_like_flavor(&dir)).then(|| Flavor::new(name, dir))
        })
        .collect();
    sort(&mut out);
    out
}

/// Flavors of `root` plus any flavor Battle.net installed elsewhere. A flavor in `root` wins
/// over one of the same name from Battle.net, but takes its client version.
pub fn all_flavors(root: Option<&Path>) -> Vec<Flavor> {
    let mut out = root.map(flavors).unwrap_or_default();
    for fl in battlenet_installs() {
        match out.iter_mut().find(|f| f.name.eq_ignore_ascii_case(&fl.name)) {
            Some(f) if same_dir(&f.dir, &fl.dir) => f.version = fl.version,
            Some(_) => {}
            None if fl.dir.is_dir() => out.push(fl),
            None => {}
        }
    }
    sort(&mut out);
    out
}

/// Finds the install root: registry first, then Battle.net, then common locations.
pub fn detect_root() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(p) = from_registry() {
        return Some(p);
    }
    if let Some(p) = battlenet_installs()
        .into_iter()
        .filter_map(|f| f.dir.parent().map(Path::to_path_buf))
        .find(|p| !flavors(p).is_empty())
    {
        return Some(p);
    }
    let mut candidates = Vec::new();
    for drive in ['C', 'D', 'E', 'F'] {
        for base in [
            r"Program Files (x86)\World of Warcraft",
            r"Program Files\World of Warcraft",
            r"Games\World of Warcraft",
            r"World of Warcraft",
        ] {
            candidates.push(PathBuf::from(format!(r"{drive}:\{base}")));
        }
    }
    candidates.into_iter().find(|p| !flavors(p).is_empty())
}

#[cfg(windows)]
fn from_registry() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for key in [
        r"SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft",
        r"SOFTWARE\Blizzard Entertainment\World of Warcraft",
    ] {
        let Ok(k) = hklm.open_subkey(key) else { continue };
        let Ok(raw) = k.get_value::<String, _>("InstallPath") else { continue };
        // InstallPath usually points at `...\World of Warcraft\_retail_\`.
        let mut path = PathBuf::from(raw.trim_end_matches(['\\', '/']));
        if path.file_name().and_then(|n| n.to_str()).is_some_and(is_flavor_name) {
            path.pop();
        }
        if !flavors(&path).is_empty() {
            return Some(path);
        }
    }
    None
}

/// Install root from the config, or auto-detected.
pub fn resolve_root(configured: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = configured {
        if flavors(p).is_empty() {
            bail!(tr!("wow.no_flavors", path = p.display()));
        }
        return Ok(p.to_path_buf());
    }
    match detect_root() {
        Some(p) => Ok(p),
        None => bail!(tr!("wow.not_found_cli")),
    }
}

// ---------------------------------------------------------------- Battle.net

/// Flavors the Battle.net agent knows about, from `%ProgramData%\Battle.net\Agent\product.db`.
pub fn battlenet_installs() -> Vec<Flavor> {
    let base = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    match fs::read(base.join(r"Battle.net\Agent\product.db")) {
        Ok(db) => installs_from_product_db(&db),
        Err(_) => Vec::new(),
    }
}

/// `product.db` is a protobuf: field 1 repeats one message per installed product, which holds
/// (among others) the install path, the flavor folder and the client version. Rather than depend
/// on the exact schema, every string of each product is collected and those are picked out.
fn installs_from_product_db(db: &[u8]) -> Vec<Flavor> {
    let Some(top) = proto_fields(db) else { return Vec::new() };
    let mut out = Vec::new();
    for (field, value) in top {
        let (1, Proto::Bytes(product)) = (field, value) else { continue };
        let mut strings = Vec::new();
        proto_strings(product, &mut strings, 0);
        let Some(path) = strings.iter().find(|s| is_abs_path(s)) else { continue };
        let version = strings.iter().find(|s| is_version(s)).cloned();
        for name in strings.iter().filter(|s| is_flavor_name(s)) {
            if !out.iter().any(|f: &Flavor| f.name == *name) {
                out.push(Flavor {
                    name: name.clone(),
                    dir: Path::new(&path.replace('/', "\\")).join(name),
                    version: version.clone(),
                });
            }
        }
    }
    out
}

/// "12.1.0.69933"
fn is_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn is_abs_path(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\')
}

enum Proto<'a> {
    Varint,
    Bytes(&'a [u8]),
}

fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*i)?;
        *i += 1;
        v |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// The fields of a protobuf message, or `None` if `b` is not one.
fn proto_fields(b: &[u8]) -> Option<Vec<(u64, Proto<'_>)>> {
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let key = varint(b, &mut i)?;
        let field = key >> 3;
        if field == 0 {
            return None;
        }
        let value = match key & 7 {
            0 => {
                varint(b, &mut i)?;
                Proto::Varint
            }
            1 => {
                i = i.checked_add(8).filter(|&e| e <= b.len())?;
                Proto::Varint
            }
            2 => {
                let len = usize::try_from(varint(b, &mut i)?).ok()?;
                let end = i.checked_add(len).filter(|&e| e <= b.len())?;
                let v = &b[i..end];
                i = end;
                Proto::Bytes(v)
            }
            5 => {
                i = i.checked_add(4).filter(|&e| e <= b.len())?;
                Proto::Varint
            }
            _ => return None,
        };
        out.push((field, value));
    }
    Some(out)
}

/// Every printable string inside a message, nested messages included.
fn proto_strings(b: &[u8], out: &mut Vec<String>, depth: usize) {
    let Some(fields) = proto_fields(b) else { return };
    for (_, value) in fields {
        let Proto::Bytes(v) = value else { continue };
        if let Ok(s) = std::str::from_utf8(v)
            && !s.is_empty()
            && !s.chars().any(char::is_control)
        {
            out.push(s.to_string());
        }
        if depth < 8 {
            proto_strings(v, out, depth + 1);
        }
    }
}

// ---------------------------------------------------------------- running game

/// A running game client and, when its path is readable, the folder it runs from.
pub struct Running {
    pub name: String,
    pub dir: Option<PathBuf>,
}

pub fn running_games() -> Vec<Running> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .filter_map(|p| {
            let running = Running {
                name: p.name().to_string_lossy().into_owned(),
                dir: p.exe().and_then(Path::parent).map(Path::to_path_buf),
            };
            is_game(&running).then_some(running)
        })
        .collect()
}

/// A known client name, or any `Wow*.exe` started from a flavor folder (a new flavor's client).
/// `WowUp.exe` and the like run from elsewhere and do not count.
fn is_game(p: &Running) -> bool {
    let name = p.name.to_lowercase();
    if GAME_EXES.contains(&name.as_str()) {
        return true;
    }
    let in_flavor = p.dir.as_deref().and_then(Path::file_name).is_some_and(|d| is_flavor_name(&d.to_string_lossy()));
    in_flavor && is_client_exe(&name)
}

/// Name of any running game process.
pub fn running_game() -> Option<String> {
    running_games().into_iter().next().map(|r| r.name)
}

/// The game process that runs `fl`. A client whose path is unknown counts for every flavor.
pub fn running_in(games: &[Running], fl: &Flavor) -> Option<String> {
    games
        .iter()
        .find(|g| g.dir.as_ref().is_none_or(|d| same_dir(d, &fl.dir)))
        .map(|g| g.name.clone())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string_field(field: u8, s: &str) -> Vec<u8> {
        let mut v = vec![(field << 3) | 2, s.len() as u8];
        v.extend_from_slice(s.as_bytes());
        v
    }

    fn message_field(field: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![(field << 3) | 2, body.len() as u8];
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn product_db_installs() {
        // Like the real file: product -> {uid, code, settings{path}, state{..., version, flavor}}.
        let product = |uid: &str, path: &str, version: &str, flavor: &str| {
            let mut body = string_field(1, uid);
            body.extend(string_field(2, "wow"));
            body.extend(message_field(3, &string_field(1, path)));
            let state = [vec![0x08, 0x01], string_field(5, version), string_field(6, flavor)].concat();
            body.extend(message_field(4, &state));
            message_field(1, &body)
        };
        let mut db = product("wow", "C:/Games/World of Warcraft", "12.1.0.69933", "_retail_");
        db.extend(product("wow_classic_beta", "D:/WoW", "1.60.1.70009", "_classic_beta_"));
        db.extend(product("wow_anniversary", "E:/Classic", "2.5.5.65000", "_anniversary_"));
        db.extend(message_field(1, &string_field(1, "battle.net")));
        db.extend(string_field(2, "agent"));
        let found = installs_from_product_db(&db);
        let got: Vec<(String, String, String)> = found
            .iter()
            .map(|f| (f.dir.to_string_lossy().replace('\\', "/"), f.version.clone().unwrap_or_default(), f.title()))
            .collect();
        let want = [
            ("C:/Games/World of Warcraft/_retail_", "12.1.0.69933", "Retail"),
            ("D:/WoW/_classic_beta_", "1.60.1.70009", "Forever Beta"),
            ("E:/Classic/_anniversary_", "2.5.5.65000", "Anniversary"),
        ]
        .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()));
        assert_eq!(got, want);
    }

    #[test]
    fn product_db_garbage_is_ignored() {
        assert!(installs_from_product_db(b"\xff\xff\xff not a protobuf").is_empty());
    }

    #[test]
    fn every_kind_of_flavor_folder_is_found() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        let dir = |name: &str| {
            let d = p.join(name);
            fs::create_dir_all(&d).unwrap();
            d
        };
        fs::create_dir_all(dir("_retail_").join("WTF")).unwrap();
        fs::create_dir_all(dir("_classic_era_").join("Interface")).unwrap();
        // Installed, never started: only Battle.net's marker, or only the client.
        fs::write(dir("_anniversary_").join(".flavor.info"), "Product Flavor!STRING:0\nwow_anniversary\n").unwrap();
        fs::write(dir("_classic_beta_").join("WowB.exe"), b"").unwrap();
        fs::write(dir("_classic_era_ptr_").join("WowClassicT.exe"), b"").unwrap();
        // A flavor nobody knows yet, with a digit in its name.
        fs::create_dir_all(dir("_dark_realm_2_").join("WTF")).unwrap();
        dir("_empty_");
        dir("Data");
        let names: Vec<String> = flavors(p).into_iter().map(|f| f.name).collect();
        assert_eq!(
            names,
            [
                "_retail_",
                "_classic_beta_",
                "_classic_era_",
                "_classic_era_ptr_",
                "_anniversary_",
                "_dark_realm_2_"
            ]
        );
    }

    #[test]
    fn running_client_blocks_only_its_flavor() {
        let retail = Flavor::new("_retail_", r"C:\Games\World of Warcraft\_retail_");
        let era = Flavor::new("_classic_era_", r"C:\Games\World of Warcraft\_classic_era_");
        let games = [Running {
            name: "Wow.exe".into(),
            dir: Some(r"c:\games\world of warcraft\_retail_\".into()),
        }];
        assert_eq!(running_in(&games, &retail).as_deref(), Some("Wow.exe"));
        assert_eq!(running_in(&games, &era), None);
        let unknown = [Running {
            name: "Wow.exe".into(),
            dir: None,
        }];
        assert!(running_in(&unknown, &era).is_some());
    }

    #[test]
    fn game_clients_are_recognized_by_their_folder() {
        let run = |name: &str, dir: Option<&str>| Running {
            name: name.into(),
            dir: dir.map(PathBuf::from),
        };
        assert!(is_game(&run("WowClassic.exe", Some(r"C:\WoW\_anniversary_"))));
        assert!(is_game(&run("WowSomethingNew.exe", Some(r"C:\WoW\_forever_"))));
        assert!(is_game(&run("Wow.exe", None)));
        assert!(!is_game(&run("WowUp.exe", Some(r"C:\Users\me\AppData\Local\Programs\WowUp"))));
        assert!(!is_game(&run("BlizzardError.exe", Some(r"C:\WoW\_retail_"))));
    }

    #[test]
    fn titles() {
        assert_eq!(flavor_title("_retail_"), "Retail");
        assert_eq!(flavor_title("_classic_era_"), "Classic Era");
        assert_eq!(flavor_title("_anniversary_"), "Anniversary");
        assert_eq!(flavor_title("_dark_realm_2_"), "Dark Realm 2");
        let mut ptr = Flavor::new("_classic_ptr_", "x");
        ptr.version = Some("5.5.1.60000".into());
        assert_eq!(ptr.title(), "Classic PTR");
        ptr.version = Some("1.61.0.71000".into());
        assert_eq!(ptr.title(), "Forever PTR");
    }
}
