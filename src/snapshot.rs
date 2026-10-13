//! Zip snapshots of WTF and AddOns.
//!
//! Layout: `<backup_dir>/<flavor>/<kind>/<YYYYmmdd-HHMMSS>[-label].zip`.
//! Paths inside the archive are relative to the flavor folder (`WTF/...`,
//! `Interface/AddOns/...`), so a restore is an extract into that folder.

use crate::wow::Flavor;
use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDateTime};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const ID_FORMAT: &str = "%Y%m%d-%H%M%S";
const ID_TS_LEN: usize = 15;
const FINGERPRINT_FILE: &str = "last.fingerprint";

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Kind {
    Wtf,
    Addons,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Wtf, Kind::Addons];

    pub fn series_name(self) -> &'static str {
        match self {
            Kind::Wtf => "wtf",
            Kind::Addons => "addons",
        }
    }

    /// Source folder relative to the flavor folder.
    pub fn source_rel(self) -> &'static str {
        match self {
            Kind::Wtf => "WTF",
            Kind::Addons => "Interface/AddOns",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub id: String,
    pub path: PathBuf,
    pub created: NaiveDateTime,
    pub size: u64,
}

pub enum Outcome {
    Created(Snapshot),
    Unchanged,
    /// The source folder does not exist (e.g. no AddOns yet).
    Missing,
}

struct SourceFile {
    abs: PathBuf,
    /// Path inside the archive, with `/`.
    rel: String,
    size: u64,
    mtime: u128,
}

pub struct Store {
    pub root: PathBuf,
    pub keep: usize,
    pub exclude_ext: Vec<String>,
}

impl Store {
    fn series_dir(&self, flavor: &Flavor, kind: Kind) -> PathBuf {
        self.root.join(&flavor.name).join(kind.series_name())
    }

    /// Where the snapshots of `kind` of `flavor` are kept.
    pub fn series_path(&self, flavor: &Flavor, kind: Kind) -> PathBuf {
        self.series_dir(flavor, kind)
    }

    /// What `kind` of `flavor` is now (paths, sizes, times), as a snapshot would record it.
    pub fn current_fingerprint(&self, flavor: &Flavor, kind: Kind) -> Result<String> {
        let files = self.collect(&flavor.dir, &flavor.dir.join(kind.source_rel()))?;
        Ok(fingerprint(&files))
    }

    /// Takes a snapshot if anything changed since the last one (or with `force`).
    pub fn backup(&self, flavor: &Flavor, kind: Kind, force: bool, label: Option<&str>) -> Result<Outcome> {
        let src = flavor.dir.join(kind.source_rel());
        if !src.is_dir() {
            return Ok(Outcome::Missing);
        }
        let files = self.collect(&flavor.dir, &src)?;
        let fp = fingerprint(&files);

        let series = self.series_dir(flavor, kind);
        fs::create_dir_all(&series).with_context(|| tr!("snapshot.create_failed", path = series.display()))?;
        let fp_path = series.join(FINGERPRINT_FILE);
        let unchanged = fs::read_to_string(&fp_path).is_ok_and(|s| s.trim() == fp);
        if unchanged && !force {
            return Ok(Outcome::Unchanged);
        }

        // Leftovers of a run that was killed while writing.
        for entry in fs::read_dir(&series)?.flatten() {
            if entry.path().extension().is_some_and(|x| x == "partial") {
                let _ = fs::remove_file(entry.path());
            }
        }
        let id = new_id(&series, label)?;
        let final_path = series.join(format!("{id}.zip"));
        let tmp_path = series.join(format!("{id}.zip.partial"));
        if let Err(e) = write_zip(&tmp_path, &files) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
        fs::rename(&tmp_path, &final_path)?;
        fs::write(&fp_path, &fp)?;

        self.prune(flavor, kind)?;
        let snap = read_snapshot(&final_path).with_context(|| tr!("snapshot.unreadable"))?;
        Ok(Outcome::Created(snap))
    }

    /// Snapshots of a series, oldest first.
    pub fn list(&self, flavor: &Flavor, kind: Kind) -> Result<Vec<Snapshot>> {
        let series = self.series_dir(flavor, kind);
        let Ok(entries) = fs::read_dir(&series) else {
            return Ok(Vec::new());
        };
        let mut out: Vec<Snapshot> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "zip"))
            .filter_map(|e| read_snapshot(&e.path()))
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    fn prune(&self, flavor: &Flavor, kind: Kind) -> Result<()> {
        if self.keep == 0 {
            return Ok(());
        }
        let list = self.list(flavor, kind)?;
        let excess = list.len().saturating_sub(self.keep);
        for snap in &list[..excess] {
            log::debug!("{}", tr!("snapshot.removing_old", path = snap.path.display()));
            fs::remove_file(&snap.path).with_context(|| tr!("snapshot.remove_failed", path = snap.path.display()))?;
        }
        Ok(())
    }

    /// Replaces the source folder with the snapshot contents.
    pub fn restore(&self, flavor: &Flavor, kind: Kind, snap: &Snapshot) -> Result<()> {
        let rel = kind.source_rel();
        let target = flavor.dir.join(rel);
        let staging = flavor.dir.join(format!(".wow_tools_restore_{}", kind.series_name()));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir_all(&staging)?;

        let mut archive = ZipArchive::new(BufReader::new(File::open(&snap.path)?)).with_context(|| tr!("snapshot.broken", path = snap.path.display()))?;
        archive.extract(&staging).with_context(|| tr!("snapshot.unpack_failed"))?;

        let restored = staging.join(rel);
        fs::create_dir_all(&restored)?;
        let old = target.with_file_name(format!("{}.wow_tools_old", target.file_name().unwrap().to_string_lossy()));
        if old.exists() {
            fs::remove_dir_all(&old)?;
        }
        if target.exists() {
            fs::rename(&target, &old).with_context(|| tr!("snapshot.move_failed", path = target.display()))?;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Err(e) = fs::rename(&restored, &target) {
            // Roll back: put the old folder back.
            if old.exists() {
                let _ = fs::rename(&old, &target);
            }
            return Err(e).with_context(|| tr!("snapshot.swap_failed"));
        }
        if old.exists() {
            fs::remove_dir_all(&old)?;
        }
        fs::remove_dir_all(&staging)?;
        Ok(())
    }

    fn collect(&self, base: &Path, src: &Path) -> Result<Vec<SourceFile>> {
        let mut out = Vec::new();
        for entry in WalkDir::new(src).sort_by_file_name() {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let excluded = path.extension().is_some_and(|ext| {
                let ext = ext.to_string_lossy();
                self.exclude_ext.iter().any(|x| x.eq_ignore_ascii_case(&ext))
            });
            if excluded {
                continue;
            }
            let meta = entry.metadata()?;
            let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            let rel = path
                .strip_prefix(base)?
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push(SourceFile {
                abs: path.to_path_buf(),
                rel,
                size: meta.len(),
                mtime,
            });
        }
        Ok(out)
    }
}

/// Picks a snapshot: `None`/`latest` is the newest; otherwise an exact id,
/// or the newest one that starts with `spec` or is older than it
/// (`20261012` = the last snapshot of that day or earlier).
pub fn select<'a>(list: &'a [Snapshot], spec: Option<&str>) -> Option<&'a Snapshot> {
    match spec {
        None | Some("latest") => list.last(),
        Some(spec) => list
            .iter()
            .find(|s| s.id == spec)
            .or_else(|| list.iter().rfind(|s| s.id.starts_with(spec) || s.id.as_str() < spec)),
    }
}

fn read_snapshot(path: &Path) -> Option<Snapshot> {
    let id = path.file_stem()?.to_str()?.to_string();
    let created = NaiveDateTime::parse_from_str(id.get(..ID_TS_LEN)?, ID_FORMAT).ok()?;
    let size = fs::metadata(path).ok()?.len();
    Some(Snapshot {
        id,
        path: path.to_path_buf(),
        created,
        size,
    })
}

fn new_id(series: &Path, label: Option<&str>) -> Result<String> {
    if let Some(l) = label
        && (l.is_empty() || !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    {
        bail!(tr!("snapshot.bad_label"));
    }
    loop {
        let ts = Local::now().format(ID_FORMAT).to_string();
        let id = match label {
            Some(l) => format!("{ts}-{l}"),
            None => ts,
        };
        // Two snapshots in the same second: wait for the next one.
        let taken = fs::read_dir(series)?
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(&id[..ID_TS_LEN]));
        if !taken {
            return Ok(id);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// Stable tree fingerprint over paths, sizes and mtimes (FNV-1a 64).
fn fingerprint(files: &[SourceFile]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for f in files {
        feed(f.rel.as_bytes());
        feed(&[0]);
        feed(&f.size.to_le_bytes());
        feed(&f.mtime.to_le_bytes());
    }
    format!("{h:016x}-{}", files.len())
}

fn write_zip(dest: &Path, files: &[SourceFile]) -> Result<()> {
    let mut zip = ZipWriter::new(BufWriter::new(File::create(dest)?));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for f in files {
        let mut src = match File::open(&f.abs) {
            Ok(file) => file,
            Err(e) => {
                log::warn!("{}", tr!("snapshot.skipping", path = f.abs.display(), error = e));
                continue;
            }
        };
        zip.start_file(f.rel.as_str(), opts)?;
        io::copy(&mut src, &mut zip).with_context(|| tr!("error.read", path = f.abs.display()))?;
    }
    let mut out = zip.finish()?;
    io::Write::flush(&mut out)?;
    Ok(())
}

pub fn human_size(bytes: u64) -> String {
    let units = [tr!("size.b"), tr!("size.kb"), tr!("size.mb"), tr!("size.gb")];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} {}", units[0])
    } else {
        format!("{v:.1} {}", units[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flavor(root: &Path) -> Flavor {
        let dir = root.join("_retail_");
        fs::create_dir_all(dir.join("WTF/Account/ACC/Realm/Char")).unwrap();
        fs::write(dir.join("WTF/Config.wtf"), "SET gxWindow \"1\"").unwrap();
        fs::write(dir.join("WTF/Account/ACC/Realm/Char/macros-cache.txt"), "MACRO 1").unwrap();
        fs::write(dir.join("WTF/Config.wtf.bak"), "junk").unwrap();
        Flavor::new("_retail_", dir)
    }

    fn store(root: &Path) -> Store {
        Store {
            root: root.join("backups"),
            keep: 3,
            exclude_ext: vec!["bak".into()],
        }
    }

    #[test]
    fn backup_skips_unchanged_and_restores() {
        let tmp = tempfile::tempdir().unwrap();
        let fl = flavor(tmp.path());
        let st = store(tmp.path());

        assert!(matches!(st.backup(&fl, Kind::Wtf, false, None).unwrap(), Outcome::Created(_)));
        assert!(matches!(st.backup(&fl, Kind::Wtf, false, None).unwrap(), Outcome::Unchanged));
        assert!(matches!(st.backup(&fl, Kind::Addons, false, None).unwrap(), Outcome::Missing));

        fs::write(fl.dir.join("WTF/Config.wtf"), "broken").unwrap();
        fs::write(fl.dir.join("WTF/extra.txt"), "new").unwrap();
        let list = st.list(&fl, Kind::Wtf).unwrap();
        st.restore(&fl, Kind::Wtf, &list[0]).unwrap();

        assert_eq!(fs::read_to_string(fl.dir.join("WTF/Config.wtf")).unwrap(), "SET gxWindow \"1\"");
        assert!(!fl.dir.join("WTF/extra.txt").exists());
        assert!(!fl.dir.join("WTF/Config.wtf.bak").exists(), "исключённые файлы не попадают в архив");
        assert!(fl.dir.join("WTF/Account/ACC/Realm/Char/macros-cache.txt").exists());
    }

    #[test]
    fn prune_keeps_last_n() {
        let tmp = tempfile::tempdir().unwrap();
        let fl = flavor(tmp.path());
        let st = Store { keep: 2, ..store(tmp.path()) };
        for _ in 0..3 {
            st.backup(&fl, Kind::Wtf, true, None).unwrap();
        }
        assert_eq!(st.list(&fl, Kind::Wtf).unwrap().len(), 2);
    }

    #[test]
    fn select_by_point_in_time() {
        let mk = |id: &str| Snapshot {
            id: id.into(),
            path: PathBuf::new(),
            created: NaiveDateTime::parse_from_str(&id[..ID_TS_LEN], ID_FORMAT).unwrap(),
            size: 0,
        };
        let list = vec![
            mk("20261010-120000"),
            mk("20261012-090000"),
            mk("20261012-180000-pre-restore"),
            mk("20261013-100000"),
        ];
        assert_eq!(select(&list, None).unwrap().id, "20261013-100000");
        assert_eq!(select(&list, Some("20261012")).unwrap().id, "20261012-180000-pre-restore");
        assert_eq!(select(&list, Some("20261011")).unwrap().id, "20261010-120000");
        assert_eq!(select(&list, Some("20261012-090000")).unwrap().id, "20261012-090000");
        assert!(select(&list, Some("2025")).is_none());
    }
}
