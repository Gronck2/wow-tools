//! Operations shared by the CLI, `watch` and the tray.

use crate::config::Config;
use crate::snapshot::{self, Kind, Outcome, Snapshot, Store};
use crate::wow::{self, Flavor};
use anyhow::{Context, Result, bail};
use chrono::NaiveDateTime;
use std::path::{Path, PathBuf};

pub struct Ctx {
    pub cfg: Config,
    pub config_path: PathBuf,
    /// Config path if given explicitly (passed on to scheduled runs).
    pub explicit_config: Option<PathBuf>,
}

impl Ctx {
    pub fn load(config_path: &Path, explicit_config: Option<&Path>) -> Result<Self> {
        Ok(Self {
            cfg: Config::load(config_path)?,
            config_path: config_path.to_path_buf(),
            explicit_config: explicit_config.map(Path::to_path_buf),
        })
    }

    pub fn store(&self) -> Store {
        Store {
            root: self.cfg.backup_dir.clone(),
            keep: self.cfg.keep,
            exclude_ext: self.cfg.exclude_ext.clone(),
        }
    }

    pub fn flavors(&self, only: Option<&str>) -> Result<Vec<Flavor>> {
        let root = match &self.cfg.wow_path {
            Some(p) => Some(wow::resolve_root(Some(p))?),
            None => wow::detect_root(),
        };
        let all = wow::all_flavors(root.as_deref());
        if all.is_empty() {
            bail!(tr!("wow.not_found"));
        }
        let wanted: Vec<String> = match only {
            Some(f) => vec![f.to_string()],
            None if !self.cfg.flavors.is_empty() => self.cfg.flavors.clone(),
            None => return Ok(all),
        };
        wanted
            .iter()
            .map(|name| {
                all.iter().find(|f| f.name.eq_ignore_ascii_case(name)).cloned().with_context(|| {
                    let names: Vec<&str> = all.iter().map(|f| f.name.as_str()).collect();
                    tr!("app.no_flavor", name = name, names = names.join(", "))
                })
            })
            .collect()
    }

    /// One flavor: the given one, the only one, or `_retail_`.
    pub fn single_flavor(&self, only: Option<&str>) -> Result<Flavor> {
        let list = self.flavors(only)?;
        if list.len() == 1 {
            return Ok(list.into_iter().next().unwrap());
        }
        if let Some(retail) = list.iter().find(|f| f.name == "_retail_") {
            return Ok(retail.clone());
        }
        let names: Vec<&str> = list.iter().map(|f| f.name.as_str()).collect();
        bail!(tr!("app.pick_flavor", names = names.join(", ")))
    }

    pub fn kinds(&self, only: Option<Kind>) -> Vec<Kind> {
        match only {
            Some(k) => vec![k],
            None if self.cfg.include_addons => Kind::ALL.to_vec(),
            None => vec![Kind::Wtf],
        }
    }

    /// Newest snapshot time over all flavors and series.
    pub fn last_backup(&self) -> Option<NaiveDateTime> {
        let store = self.store();
        self.flavors(None)
            .ok()?
            .iter()
            .flat_map(|fl| Kind::ALL.map(|k| store.list(fl, k).ok().and_then(|l| l.last().map(|s| s.created))))
            .flatten()
            .max()
    }
}

#[derive(Debug, Default)]
pub struct Report {
    /// "_retail_ WTF: 20261012-003548 (12.8 МБ)" per new snapshot.
    pub created: Vec<String>,
    /// Set when the run was skipped because the game is running.
    pub skipped_game: Option<String>,
    pub errors: Vec<String>,
}

/// Snapshots every selected flavor and series. `force` ignores the fingerprint and the
/// running game; `ignore_game` only the running game (the tray "backup now").
pub fn backup_all(ctx: &Ctx, flavor: Option<&str>, kind: Option<Kind>, force: bool, ignore_game: bool) -> Result<Report> {
    let mut report = Report::default();
    // Only the flavor whose client runs waits: retail running does not hold back Classic.
    let games = if !force && !ignore_game && ctx.cfg.skip_while_running {
        wow::running_games()
    } else {
        Vec::new()
    };
    let store = ctx.store();
    for fl in ctx.flavors(flavor)? {
        if let Some(p) = wow::running_in(&games, &fl) {
            log::info!("{}", tr!("app.skip_running", flavor = fl.name, game = p));
            report.skipped_game = Some(p);
            continue;
        }
        for k in ctx.kinds(kind) {
            let what = format!("{} {}", fl.name, k.source_rel());
            match store.backup(&fl, k, force, None) {
                Ok(Outcome::Created(s)) => {
                    let line = format!("{what}: {} ({})", s.id, snapshot::human_size(s.size));
                    log::info!("{}", tr!("app.snapshot_made", line = line));
                    report.created.push(line);
                }
                Ok(Outcome::Unchanged) => log::info!("{}", tr!("app.unchanged", what = what)),
                Ok(Outcome::Missing) => log::debug!("{}", tr!("app.no_folder", what = what)),
                Err(e) => {
                    log::error!("{what}: {e:#}");
                    report.errors.push(format!("{what}: {e:#}"));
                }
            }
        }
    }
    Ok(report)
}

/// Fails while the client of `fl` runs: it would overwrite the files on exit.
pub fn ensure_game_closed(fl: &Flavor) -> Result<()> {
    if let Some(p) = wow::running_in(&wow::running_games(), fl) {
        bail!(tr!("app.close_game", game = p, flavor = fl.title()));
    }
    Ok(())
}

pub type Plan = Vec<(Kind, Snapshot)>;

/// Which snapshot of each series a restore to `spec` would use.
pub fn restore_plan(ctx: &Ctx, flavor: Option<&str>, spec: Option<&str>, kind: Option<Kind>) -> Result<(Flavor, Plan)> {
    let fl = ctx.single_flavor(flavor)?;
    let store = ctx.store();
    let mut plan = Vec::new();
    for k in kind.map_or(Kind::ALL.to_vec(), |k| vec![k]) {
        let list = store.list(&fl, k)?;
        match snapshot::select(&list, spec) {
            Some(s) => plan.push((k, s.clone())),
            None if kind.is_some() => bail!(tr!("app.no_snapshot", what = k.source_rel())),
            None => log::warn!("{}", tr!("app.no_snapshot_skip", what = k.source_rel())),
        }
    }
    if plan.is_empty() {
        bail!(tr!("app.nothing_to_restore"));
    }
    Ok((fl, plan))
}

/// Saves the current state as `pre-restore` snapshots, then restores the plan.
pub fn restore_apply(ctx: &Ctx, fl: &Flavor, plan: &Plan) -> Result<()> {
    ensure_game_closed(fl)?;
    let store = ctx.store();
    for (k, s) in plan {
        if let Outcome::Created(pre) = store.backup(fl, *k, true, Some("pre-restore"))? {
            log::info!("{}", tr!("app.saved_current", flavor = fl.name, what = k.source_rel(), id = pre.id));
        }
        store.restore(fl, *k, s)?;
        log::info!("{}", tr!("app.restored_from", flavor = fl.name, what = k.source_rel(), id = s.id));
    }
    Ok(())
}

/// Restores every series of `flavor` (or only `kind`) to the point in time `spec`; returns a short summary.
pub fn restore_point(ctx: &Ctx, flavor: &str, spec: &str, kind: Option<Kind>) -> Result<String> {
    let (fl, plan) = restore_plan(ctx, Some(flavor), Some(spec), kind)?;
    restore_apply(ctx, &fl, &plan)?;
    let lines: Vec<String> = plan.iter().map(|(k, s)| format!("{} ← {}", k.source_rel(), s.id)).collect();
    Ok(format!("{}\n{}", fl.title(), lines.join("\n")))
}

/// Updates addons through the chosen addon manager app; first snapshots the addons of every selected
/// flavor (when they changed since the last snapshot) so a bad update can be rolled back.
pub fn update_addons(ctx: &Ctx, manual: bool) -> Result<String> {
    // Refused before anything is recorded: nothing was tried.
    if ctx.cfg.update_skip_in_game
        && let Some(game) = crate::wow::running_game()
    {
        bail!(tr!("app.no_update_in_game", game = game));
    }
    let app = crate::updater::chosen(ctx.cfg.update_app);
    let result = app.with_context(|| tr!("app.no_addon_app")).and_then(|app| run_update(ctx, app));
    crate::updater::record(crate::updater::Entry {
        time: chrono::Local::now().naive_local(),
        app: app.map_or("—", |a| a.title()).to_string(),
        ok: result.is_ok(),
        text: match &result {
            Ok(summary) => summary.clone(),
            Err(e) => format!("{e:#}"),
        },
        manual,
    });
    result
}

fn run_update(ctx: &Ctx, app: crate::updater::AddonApp) -> Result<String> {
    let store = ctx.store();
    for fl in ctx.flavors(None)? {
        if let Outcome::Created(s) = store
            .backup(&fl, Kind::Addons, false, Some("pre-update"))
            .with_context(|| tr!("app.pre_update_failed", flavor = fl.name))?
        {
            log::info!("{}", tr!("app.pre_update_saved", flavor = fl.name, id = s.id));
        }
    }
    let summary = app.update((ctx.cfg.wago_weakauras, ctx.cfg.wago_ui_packs))?.summary();
    log::info!("{}: {}", app.title(), summary.replace('\n', " | "));
    Ok(summary)
}
