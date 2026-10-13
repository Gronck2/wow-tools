//! The background loop behind `watch` and the tray: automatic snapshots (after the game closes,
//! or once on each scheduled day), plus on-demand requests.

use crate::app::{self, Ctx, Report};
use crate::chars::Character;
use crate::config::When;
use crate::maintenance::{self, Item, Patch};
use crate::profiles::{self, Profile};
use crate::snapshot::Kind;
use crate::{updater, wow};
use anyhow::Result;
use chrono::{Local, NaiveDateTime};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(15);
/// Lets the client finish writing WTF after it exits.
const AFTER_EXIT_DELAY: Duration = Duration::from_secs(5);

pub enum Request {
    BackupNow,
    /// Update addons through CurseForge now.
    UpdateAddons,
    /// Restore every series of a flavor to this snapshot id / point in time.
    Restore {
        flavor: String,
        id: String,
        /// Only this series (the "before addon update" points); `None` = WTF and addons.
        kind: Option<Kind>,
    },
    /// Tidy one thing up (`maintenance::clean`).
    Cleanup(Item),
    /// Save a character's interface as a profile (`profiles::save`).
    SaveProfile {
        flavor: String,
        character: Character,
    },
    /// Put a profile on a character (`profiles::apply`).
    ApplyProfile {
        profile: Profile,
        target: Character,
    },
}

/// What a busy worker is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Work {
    #[default]
    Backup,
    Addons,
    Restore,
    Cleanup,
    Profile,
}

pub enum Event {
    Busy(Work),
    Backup {
        manual: bool,
        result: Result<Report>,
    },
    Restore(Result<String>),
    /// An addon update finished: its summary or the error.
    Addons {
        manual: bool,
        result: Result<String>,
    },
    /// The game process started (`Some(name)`) or exited (`None`).
    Game(Option<String>),
    /// Battle.net patched these flavors (snapshots taken before their first start).
    Patch(Vec<Patch>),
    Cleanup(Result<String>),
    /// A profile job finished: its summary (`notice`: nothing was done, worth a look) or the error.
    Profile {
        result: Result<String>,
        notice: bool,
    },
}

/// How often client versions are compared with the last ones seen.
const PATCH_CHECK: Duration = Duration::from_secs(60);

/// How long a scheduled snapshot waits before trying again after it could not finish
/// (the game was running or something failed).
const RETRY: Duration = Duration::from_secs(10 * 60);

/// Runs until `requests` is disconnected. The config is re-read every round,
/// so edits apply without a restart. `interval_override` (`watch --interval`) replaces
/// the configured schedule with a plain timer.
pub fn run(config_path: &Path, explicit_config: Option<&Path>, interval_override: Option<u64>, requests: &Receiver<Request>, mut emit: impl FnMut(Event)) {
    let mut ctx = match Ctx::load(config_path, explicit_config) {
        Ok(ctx) => ctx,
        Err(e) => {
            emit(Event::Backup { manual: false, result: Err(e) });
            Ctx {
                cfg: Default::default(),
                config_path: config_path.to_path_buf(),
                explicit_config: explicit_config.map(Path::to_path_buf),
            }
        }
    };
    let mut was_running: Option<String> = None;
    let mut next = Instant::now();
    // When the last snapshot was made; a scheduled moment after it is due (and caught up if missed).
    let mut last_done: Option<NaiveDateTime> = ctx.last_backup();

    let mut retry_at = Instant::now();
    let mut last_update = updater::last_run();
    let mut update_retry_at = Instant::now();
    let mut patch_check_at = Instant::now();
    loop {
        match Ctx::load(config_path, explicit_config) {
            Ok(fresh) => ctx = fresh,
            Err(e) => log::warn!("{}", tr!("watch.config_kept", error = format!("{e:#}"))),
        }

        let running = wow::running_game();
        if running.is_some() != was_running.is_some() {
            emit(Event::Game(running.clone()));
        }
        let just_closed = was_running.is_some() && running.is_none();
        was_running = running;
        if just_closed {
            // A scheduled snapshot held back by the game goes right after it.
            retry_at = Instant::now();
        }

        // A patch is caught before the first start on it, while WTF still holds the old state.
        if Instant::now() >= patch_check_at {
            patch_check_at = Instant::now() + PATCH_CHECK;
            let patches = maintenance::check_patches(&ctx);
            if !patches.is_empty() {
                emit(Event::Patch(patches));
            }
        }

        let now = Local::now().naive_local();
        let due = match interval_override {
            Some(minutes) => {
                let now = Instant::now();
                let due = just_closed || now >= next;
                if due {
                    next = now + Duration::from_secs(60 * minutes.max(1));
                }
                due
            }
            None => match ctx.cfg.backup_when {
                When::GameExit => just_closed,
                When::Schedule => ctx.cfg.schedule_due(now, last_done) && Instant::now() >= retry_at,
            },
        };
        if due {
            if just_closed {
                log::info!("{}", tr!("watch.game_closed"));
                std::thread::sleep(AFTER_EXIT_DELAY);
            }
            emit(Event::Busy(Work::Backup));
            let result = app::backup_all(&ctx, None, None, false, false);
            match &result {
                Ok(r) if r.errors.is_empty() && r.skipped_game.is_none() => last_done = Some(now),
                _ => retry_at = Instant::now() + RETRY,
            }
            emit(Event::Backup { manual: false, result });
        }

        // Automatic addon update (schedule or interval): with `update_skip_in_game` only with the game
        // closed (addons are swapped under it otherwise); then it runs right after the game exits.
        let game_ok = was_running.is_none() || !ctx.cfg.update_skip_in_game;
        if ctx.cfg.update_due(now, last_update) && game_ok && Instant::now() >= update_retry_at {
            emit(Event::Busy(Work::Addons));
            let result = app::update_addons(&ctx, false);
            match &result {
                // `update_addons` saved its start time.
                Ok(_) => last_update = updater::last_run(),
                Err(e) => {
                    log::error!("{}", tr!("watch.update_failed", error = format!("{e:#}")));
                    update_retry_at = Instant::now() + 3 * RETRY;
                }
            }
            emit(Event::Addons { manual: false, result });
        }

        match requests.recv_timeout(POLL) {
            Ok(Request::BackupNow) => {
                emit(Event::Busy(Work::Backup));
                let result = app::backup_all(&ctx, None, None, false, true);
                if result.as_ref().is_ok_and(|r| r.errors.is_empty() && r.skipped_game.is_none()) {
                    last_done = Some(Local::now().naive_local());
                }
                emit(Event::Backup { manual: true, result });
            }
            Ok(Request::UpdateAddons) => {
                emit(Event::Busy(Work::Addons));
                let result = app::update_addons(&ctx, true);
                if result.is_ok() {
                    last_update = updater::last_run();
                }
                emit(Event::Addons { manual: true, result });
            }
            Ok(Request::Cleanup(item)) => {
                emit(Event::Busy(Work::Cleanup));
                let result = maintenance::clean(&ctx, item);
                if let Err(e) = &result {
                    log::error!("{e:#}");
                }
                emit(Event::Cleanup(result));
            }
            Ok(Request::SaveProfile { flavor, character }) => {
                emit(Event::Busy(Work::Profile));
                let result = ctx.single_flavor(Some(&flavor)).and_then(|fl| profiles::save(&ctx, &fl, &character));
                if let Err(e) = &result {
                    log::error!("{e:#}");
                }
                let notice = result.as_ref().is_ok_and(|s| !s.new);
                emit(Event::Profile {
                    result: result.map(|s| s.summary),
                    notice,
                });
            }
            Ok(Request::ApplyProfile { profile, target }) => {
                emit(Event::Busy(Work::Profile));
                let result = ctx
                    .single_flavor(Some(&profile.meta.flavor))
                    .and_then(|fl| profiles::apply(&ctx, &fl, &profile, &target));
                if let Err(e) = &result {
                    log::error!("{e:#}");
                }
                emit(Event::Profile { result, notice: false });
            }
            Ok(Request::Restore { flavor, id, kind }) => {
                emit(Event::Busy(Work::Restore));
                let result = app::restore_point(&ctx, &flavor, &id, kind);
                if let Err(e) = &result {
                    log::error!("{}", tr!("watch.restore_failed", error = format!("{e:#}")));
                }
                emit(Event::Restore(result));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
