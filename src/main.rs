#[macro_use]
mod i18n;
mod app;
mod chars;
mod config;
mod curseforge;
#[cfg(windows)]
mod gfx;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod panel;
mod schedule;
mod selfupdate;
#[cfg(windows)]
mod settings;
mod snapshot;
#[cfg(windows)]
mod tray;
mod updater;
mod wago;
mod watch;
mod wow;
mod wowup;

use anyhow::{Context, Result, bail};
use app::Ctx;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use config::Config;
use snapshot::{Kind, Outcome};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

/// `--help` texts come from `locales/` (`localize`); the English here is what clap gets first.
#[derive(Parser)]
#[command(version, about = "WoW Tools: backups of WTF and addons, settings between characters")]
struct Cli {
    /// Config path (default %APPDATA%\WoW Tools\config\config.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Write the log to a file (%LOCALAPPDATA%\WoW Tools\data\wow-tools.log), not the console
    #[arg(long, global = true)]
    log_to_file: bool,
    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Tray icon: background backups, restore, settings
    Tray {
        /// Wait for this process to exit first (the tray restarting after an update)
        #[arg(long, hide = true)]
        wait_pid: Option<u32>,
    },
    /// Create the config (finds the game by itself)
    Init {
        /// WoW folder (the one with _retail_, _classic_, ...)
        #[arg(long)]
        wow_path: Option<PathBuf>,
        /// Where to keep backups
        #[arg(long)]
        backup_dir: Option<PathBuf>,
        /// Overwrite an existing config
        #[arg(long)]
        force: bool,
    },
    /// Show the install found and the current settings
    Status,
    /// Take a snapshot (only when something changed, or --force)
    Backup {
        /// Game version: _retail_, _classic_, ...
        #[arg(long)]
        flavor: Option<String>,
        /// Only WTF or only addons
        #[arg(long)]
        kind: Option<Kind>,
        /// Snapshot even with no changes and with the game running
        #[arg(long)]
        force: bool,
    },
    /// List snapshots
    List {
        /// Game version: _retail_, _classic_, ...
        #[arg(long)]
        flavor: Option<String>,
        /// Only WTF or only addons
        #[arg(long)]
        kind: Option<Kind>,
    },
    /// Restore from a snapshot (the current state is saved first)
    Restore {
        /// Snapshot id, a date prefix (20261012 — the latest of that day or earlier) or latest
        snapshot: Option<String>,
        /// Game version: _retail_, _classic_, ...
        #[arg(long)]
        flavor: Option<String>,
        /// Only WTF or only addons
        #[arg(long)]
        kind: Option<Kind>,
        /// Do not ask for confirmation
        #[arg(short, long)]
        yes: bool,
    },
    /// List characters
    Chars {
        /// Game version: _retail_, _classic_, ...
        #[arg(long)]
        flavor: Option<String>,
    },
    /// Copy settings from one character to others
    CopyChar {
        /// Source: Realm/Name or ACCOUNT/Realm/Name
        #[arg(long)]
        from: String,
        /// Target (may be given several times)
        #[arg(long, required = true)]
        to: Vec<String>,
        /// What to copy (everything by default)
        #[arg(long, value_delimiter = ',')]
        parts: Vec<chars::Part>,
        /// Game version: _retail_, _classic_, ...
        #[arg(long)]
        flavor: Option<String>,
        /// Only show what would be copied
        #[arg(long)]
        dry_run: bool,
    },
    /// Run in the console without the tray: snapshots as set in the settings
    Watch {
        /// Instead of the settings: a snapshot every N minutes and after the game closes
        #[arg(long)]
        interval: Option<u64>,
    },
    /// Update addons through the chosen app (started without a window and closed)
    UpdateAddons,
    /// Check GitHub for a new version of WoW Tools and install it
    SelfUpdate {
        /// Only check, do not install
        #[arg(long)]
        check: bool,
    },
    /// Automatic start
    Schedule {
        #[command(subcommand)]
        action: ScheduleCmd,
    },
    /// Draws the panel or the menu with demo data into a PNG (for checking the look)
    #[command(hide = true)]
    UiSnapshot {
        out: PathBuf,
        /// panel, busy, game, error, confirm, empty, busy-addons, addons, addons-busy, addons-tip, addons-empty, menu, settings, settings-backup, settings-exit, settings-addons, settings-every, settings-one or settings-wago
        #[arg(default_value = "panel")]
        view: String,
        #[arg(long)]
        dark: bool,
        /// Interface language: ru, en, de, fr, es
        #[arg(long)]
        lang: Option<String>,
    },
}

#[derive(Subcommand)]
enum ScheduleCmd {
    /// Task Scheduler task: `backup` every N minutes (once a day by default)
    Interval {
        /// Minutes between runs
        #[arg(long)]
        every: Option<u64>,
    },
    /// Start the tray when signing in to Windows
    Logon {
        /// Start `watch` without an icon instead of the tray
        #[arg(long)]
        headless: bool,
    },
    /// Remove the task and the autostart
    Remove,
    /// Show what is set up
    Status,
}

/// The language for this run: from `--config` or the default config, else Windows'.
fn early_language() -> Option<i18n::Lang> {
    let args: Vec<String> = std::env::args().collect();
    let explicit = args.iter().position(|a| a == "--config").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let explicit = explicit.or_else(|| args.iter().find_map(|a| a.strip_prefix("--config=")).map(PathBuf::from));
    let path = explicit.unwrap_or_else(config::default_config_path);
    Config::load(&path).ok().and_then(|c| c.language)
}

/// Puts the interface language into `--help`: `cli.<command path>.about` for commands,
/// `cli.<command path>.<arg>` for arguments (`cli.about`, `cli.config` at the top), and the
/// headings and the built-in help/version flags.
fn localize(cmd: clap::Command, path: &str) -> clap::Command {
    use clap::{Arg, ArgAction};
    let key = |k: &str| if path.is_empty() { format!("cli.{k}") } else { format!("cli.{path}.{k}") };
    let mut cmd = cmd
        .help_template(format!("{{about-with-newline}}\n{}: {{usage}}\n\n{{all-args}}{{after-help}}", tr!("cli.usage")))
        .subcommand_help_heading(tr!("cli.commands"))
        .disable_help_subcommand(true)
        .disable_help_flag(true)
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .help(tr!("cli.help"))
                .help_heading(tr!("cli.options")),
        );
    if path.is_empty() {
        cmd = cmd.about(tr!("cli.about")).after_help(tr!("cli.after_help")).disable_version_flag(true).arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .action(ArgAction::Version)
                .help(tr!("cli.version"))
                .help_heading(tr!("cli.options")),
        );
    } else if let Some(about) = i18n::lookup(&format!("cli.{path}.about")) {
        cmd = cmd.about(about);
    }
    let ids: Vec<String> = cmd
        .get_arguments()
        .map(|a| a.get_id().to_string())
        .filter(|id| id != "help" && id != "version")
        .collect();
    for id in ids {
        let help = i18n::lookup(&key(&id));
        cmd = cmd.mut_arg(&id, |a| {
            let heading = if a.is_positional() { tr!("cli.arguments") } else { tr!("cli.options") };
            let a = a.help_heading(heading);
            match help {
                Some(h) => a.help(h),
                None => a,
            }
        });
    }
    let subs: Vec<String> = cmd.get_subcommands().map(|s| s.get_name().to_string()).collect();
    for name in subs {
        let sub_path = if path.is_empty() { name.clone() } else { format!("{path}.{name}") };
        cmd = cmd.mut_subcommand(&name, |s| localize(s, &sub_path));
    }
    cmd
}

fn main() {
    i18n::set(early_language());
    let matches = localize(Cli::command(), "").get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    // Before logging: the log file lives in the folder that may be moved.
    let migrated = config::migrate_legacy();
    if let Err(e) = init_logging(&cli) {
        eprintln!("{}", tr!("out.log_failed", error = format!("{e:#}")));
    }
    if let Err(e) = migrated {
        log::warn!("{}", tr!("out.migrate_failed", error = format!("{e:#}")));
    }
    if let Err(e) = run(cli) {
        log::error!("{e:#}");
        std::process::exit(1);
    }
}

fn init_logging(cli: &Cli) -> Result<()> {
    let level = if cli.verbose { "debug" } else { "info" };
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level));
    // The tray has no console to write to.
    let to_file = cli.log_to_file || matches!(cli.cmd, None | Some(Cmd::Tray { .. }));
    if to_file {
        let path = config::default_log_path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        // Simple rotation so the log does not grow forever.
        if fs::metadata(&path).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
            let _ = fs::rename(&path, path.with_extension("log.old"));
        }
        let file = fs::OpenOptions::new().create(true).append(true).open(&path)?;
        builder
            .target(env_logger::Target::Pipe(Box::new(file)))
            .write_style(env_logger::WriteStyle::Never)
            // Local time: snapshot ids are local too, UTC here only confuses.
            .format(|buf, rec| writeln!(buf, "{} {:<5} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), rec.level(), rec.args()));
    } else {
        builder.format_timestamp(None).format_target(false);
    }
    builder.init();
    Ok(())
}

fn run(cli: Cli) -> Result<()> {
    let config_path = cli.config.clone().unwrap_or_else(config::default_config_path);
    let ctx = Ctx::load(&config_path, cli.config.as_deref())?;

    match cli.cmd.unwrap_or(Cmd::Tray { wait_pid: None }) {
        Cmd::Tray { wait_pid } => {
            if let Some(pid) = wait_pid {
                selfupdate::wait_for_exit(pid);
            }
            cmd_tray(ctx)
        }
        Cmd::Init { wow_path, backup_dir, force } => cmd_init(ctx, wow_path, backup_dir, force),
        Cmd::Status => cmd_status(&ctx),
        Cmd::Backup { flavor, kind, force } => cmd_backup(&ctx, flavor.as_deref(), kind, force),
        Cmd::List { flavor, kind } => cmd_list(&ctx, flavor.as_deref(), kind),
        Cmd::Restore { snapshot, flavor, kind, yes } => cmd_restore(&ctx, snapshot.as_deref(), flavor.as_deref(), kind, yes),
        Cmd::Chars { flavor } => cmd_chars(&ctx, flavor.as_deref()),
        Cmd::CopyChar {
            from,
            to,
            parts,
            flavor,
            dry_run,
        } => cmd_copy_char(&ctx, &from, &to, &parts, flavor.as_deref(), dry_run),
        Cmd::Watch { interval } => cmd_watch(&ctx, interval),
        Cmd::UpdateAddons => {
            println!("{}", app::update_addons(&ctx, true)?);
            Ok(())
        }
        Cmd::SelfUpdate { check } => cmd_self_update(check),
        Cmd::Schedule { action } => cmd_schedule(&ctx, action),
        Cmd::UiSnapshot { out, view, dark, lang } => {
            if let Some(code) = lang {
                let lang = i18n::Lang::ALL.into_iter().find(|l| format!("{l:?}").eq_ignore_ascii_case(&code));
                i18n::set(Some(lang.with_context(|| format!("--lang: {code}?"))?));
            }
            cmd_ui_snapshot(&out, &view, dark)
        }
    }
}

fn cmd_self_update(check_only: bool) -> Result<()> {
    let Some(release) = selfupdate::check()? else {
        println!("{}", tr!("self_update.up_to_date", version = selfupdate::current()));
        return Ok(());
    };
    println!("{}", tr!("self_update.cli_available", version = &release.version, url = &release.page));
    if check_only {
        return Ok(());
    }
    println!("{}", tr!("self_update.installing", version = &release.version));
    let new = selfupdate::download(&release)?;
    selfupdate::install(&new, &selfupdate::exe_path()?)?;
    println!("{}", tr!("self_update.cli_installed", version = &release.version));
    Ok(())
}

#[cfg(windows)]
fn cmd_tray(ctx: Ctx) -> Result<()> {
    tray::run(ctx)
}

#[cfg(not(windows))]
fn cmd_tray(_: Ctx) -> Result<()> {
    bail!(tr!("out.tray_windows_only"))
}

#[cfg(windows)]
fn cmd_ui_snapshot(out: &std::path::Path, view: &str, dark: bool) -> Result<()> {
    use chrono::{Duration, Local};
    use tray::{Phase, RestorePoint, Status};
    let path = out.to_string_lossy();
    // A newer release, for the views that show one.
    let new_release = || tray::SelfUpdate {
        available: Some(selfupdate::Release {
            version: "0.2.0".into(),
            page: String::new(),
            asset: None,
        }),
        checked: Some(Local::now().naive_local()),
        ..Default::default()
    };
    if view == "menu" || view == "menu-update" {
        return Ok(menu::snapshot(&path, dark, (view == "menu-update").then_some("0.2.0"))?);
    }
    if view.starts_with("settings") {
        let model = settings::Model {
            cfg: Config {
                flavors: vec!["_retail_".into(), "_classic_beta_".into(), "_classic_era_".into()],
                backup_when: if view == "settings-exit" {
                    config::When::GameExit
                } else {
                    config::When::Schedule
                },
                backup_days: vec![config::Day::Mon, config::Day::Wed, config::Day::Fri, config::Day::Sat],
                update_addons: view != "settings-exit",
                update_days: vec![config::Day::Tue, config::Day::Sat],
                update_app: (view == "settings-wago").then_some(updater::AddonApp::Wago),
                update_when: if view == "settings-every" {
                    config::UpdateWhen::Interval
                } else {
                    config::UpdateWhen::Schedule
                },
                ..Config::default()
            },
            detected: Some(r"C:\Games\World of Warcraft".into()),
            flavors: ["_retail_", "_ptr_", "_classic_", "_classic_beta_", "_classic_era_", "_anniversary_"]
                .map(|n| wow::Flavor {
                    version: (n == "_classic_beta_").then(|| "1.60.1.70009".into()),
                    ..wow::Flavor::new(n, "")
                })
                .to_vec(),
            autostart: true,
            apps: if view == "settings-one" {
                vec![updater::AddonApp::CurseForge]
            } else {
                vec![updater::AddonApp::CurseForge, updater::AddonApp::WowUp, updater::AddonApp::Wago]
            },
            weakauras: true,
            last_update: Some(chrono::Local::now().naive_local() - chrono::Duration::hours(20)),
            message: None,
            update: if view == "settings-update" {
                new_release()
            } else {
                tray::SelfUpdate {
                    checked: Some(Local::now().naive_local() - Duration::hours(3)),
                    ..Default::default()
                }
            },
        };
        let tab = match view {
            "settings-backup" | "settings-exit" => settings::Tab::Backup,
            "settings-addons" | "settings-every" | "settings-one" | "settings-wago" => settings::Tab::Addons,
            _ => settings::Tab::General,
        };
        return Ok(settings::snapshot(&path, dark, &model, tab)?);
    }
    let now = Local::now().naive_local();
    let point = |flavor: &str, hours: i64, label: &str, addons: Option<u64>| RestorePoint {
        flavor: flavor.into(),
        flavor_title: Some(wow::flavor_title(flavor)),
        id: format!("{}{label}", (now - Duration::hours(hours)).format("%Y%m%d-%H%M%S")),
        created: now - Duration::hours(hours),
        wtf_size: 13_400_000,
        addons_size: addons,
        running: false,
        kind: None,
    };
    let mut demo = panel::View {
        status: Status {
            phase: Phase::Idle,
            work: watch::Work::Backup,
            last: Some(now),
            game: None,
        },
        points: vec![
            point("_retail_", 0, "", Some(75_300_000)),
            RestorePoint {
                kind: Some(Kind::Addons),
                ..point("_retail_", 1, "-pre-update", Some(71_800_000))
            },
            point("_classic_era_", 2, "", Some(21_800_000)),
            point("_retail_", 3, "-pre-restore", Some(75_300_000)),
            point("_retail_", 50, "", None),
        ],
        autostart: true,
        schedule: Config {
            backup_when: config::When::Schedule,
            backup_days: vec![config::Day::Mon, config::Day::Wed, config::Day::Fri],
            ..Config::default()
        }
        .schedule_text(),
        update_app: Some("CurseForge"),
        update_text: tray::upper_first(&config::every_text(60)),
        update_waits_for_game: false,
        last_update: Some(now - Duration::minutes(25)),
        updates: [
            (
                25,
                "CurseForge",
                true,
                true,
                tr!("addons.updated", n = 3, names = "BigWigs, Details! Damage Meter, ElvUI"),
            ),
            (85, "CurseForge", true, false, tr!("addons.none").to_string()),
            (
                1450,
                "WowUp",
                false,
                false,
                tr!(
                    "wowup.failed_all",
                    app = "WowUp",
                    names = "BugSack",
                    why = tr!("wowup.wago_denied", app = "WowUp")
                ),
            ),
            (1500, "WowUp", true, false, tr!("addons.updated", n = 1, names = "WeakAuras")),
        ]
        .map(|(min, app, ok, manual, text)| updater::Entry {
            time: now - Duration::minutes(min),
            app: app.into(),
            ok,
            text,
            manual,
        })
        .to_vec(),
        self_update: if view.starts_with("update") { new_release() } else { Default::default() },
    };
    let mut confirm = None;
    let tab = if view.starts_with("addons") { panel::Tab::Addons } else { panel::Tab::Backup };
    match view {
        "busy" => demo.status.phase = Phase::Busy,
        "addons-busy" | "busy-addons" => {
            demo.status.phase = Phase::Busy;
            demo.status.work = watch::Work::Addons;
        }
        "addons-empty" => {
            demo.updates.clear();
            demo.last_update = None;
        }
        "game" => {
            demo.status.game = Some("Wow.exe".into());
            for p in demo.points.iter_mut().filter(|p| p.flavor == "_retail_") {
                p.running = true;
            }
        }
        "error" => demo.status.phase = Phase::Error(format!("_retail_ WTF: {} (os error 5)", tr!("snapshot.remove_failed", path = "Config.wtf"))),
        "confirm" => confirm = Some(1),
        "empty" => {
            demo.points.clear();
            demo.status.last = None;
            demo.autostart = false;
        }
        _ => {}
    }
    let tip = (view == "addons-tip").then_some(2);
    Ok(panel::snapshot(
        &path,
        dark,
        &demo,
        tab,
        view == "panel" || view == "addons",
        confirm,
        tip,
        view == "update-tip",
    )?)
}

#[cfg(not(windows))]
fn cmd_ui_snapshot(_: &std::path::Path, _: &str, _: bool) -> Result<()> {
    bail!(tr!("out.windows_only"))
}

fn cmd_init(ctx: Ctx, wow_path: Option<PathBuf>, backup_dir: Option<PathBuf>, force: bool) -> Result<()> {
    if ctx.config_path.exists() && !force {
        bail!(tr!("out.config_exists", path = ctx.config_path.display()));
    }
    let mut cfg = Config::default();
    let root = match wow_path {
        Some(p) => p,
        None => wow::detect_root().with_context(|| tr!("out.wow_not_found"))?,
    };
    wow::resolve_root(Some(&root))?;
    cfg.wow_path = Some(root);
    if let Some(dir) = backup_dir {
        cfg.backup_dir = dir;
    }
    cfg.save(&ctx.config_path)?;
    println!("{}", tr!("out.config_written", path = ctx.config_path.display()));
    println!("{}", toml::to_string_pretty(&cfg)?);
    Ok(())
}

fn cmd_status(ctx: &Ctx) -> Result<()> {
    let missing = if ctx.config_path.exists() { "" } else { tr!("out.config_missing") };
    println!("{:<11}{}{missing}", tr!("out.config"), ctx.config_path.display());
    println!(
        "{:<11}{}",
        tr!("out.backups"),
        tr!("out.backups_keep", path = ctx.cfg.backup_dir.display(), n = ctx.cfg.keep)
    );
    let root = match &ctx.cfg.wow_path {
        Some(p) => wow::resolve_root(Some(p)).map(Some),
        None => Ok(wow::detect_root()),
    };
    match root {
        Ok(root) => {
            let all = wow::all_flavors(root.as_deref());
            match &root {
                Some(r) => println!("{:<11}{}", "WoW:", r.display()),
                None if all.is_empty() => println!("{:<11}{}", "WoW:", tr!("out.wow_missing")),
                None => println!("{:<11}{}", "WoW:", tr!("out.wow_from_bnet")),
            }
            let selected: Vec<String> = ctx.flavors(None).map(|l| l.into_iter().map(|f| f.name).collect()).unwrap_or_default();
            let store = ctx.store();
            for fl in all {
                let mark = if selected.contains(&fl.name) { '*' } else { ' ' };
                let n_chars = chars::list(&fl.wtf_dir()).len();
                print!("{mark} {:<18} {}", fl.name, tr!("out.chars_count", n = format!("{n_chars:<4}")));
                for kind in Kind::ALL {
                    let list = store.list(&fl, kind)?;
                    match list.last() {
                        Some(s) => print!(" {}", tr!("out.series", kind = kind.series_name(), n = list.len(), last = s.created)),
                        None => print!(" {}", tr!("out.series_none", kind = kind.series_name())),
                    }
                }
                if root.as_deref().is_none_or(|r| fl.dir.parent() != Some(r)) {
                    print!("  ({})", fl.dir.display());
                }
                println!();
            }
            println!("  {}", tr!("out.selected_mark"));
        }
        Err(e) => println!("{:<11}{}", "WoW:", tr!("out.wow_error", error = format!("{e:#}"))),
    }
    for g in wow::running_games() {
        let line = match g.dir {
            Some(d) => tr!("out.game_running_in", game = g.name, path = d.display()),
            None => tr!("out.game_running", game = g.name),
        };
        println!("{:<11}{line}", tr!("out.game"));
    }
    if wow::running_game().is_none() {
        println!("{:<11}{}", tr!("out.game"), tr!("out.game_not_running"));
    }
    let chosen = updater::chosen(ctx.cfg.update_app);
    for app in updater::AddonApp::installed() {
        let mark = if Some(app) == chosen { "*" } else { " " };
        match app.describe() {
            Ok(lines) if lines.is_empty() => println!("{mark} {}: {}", app.title(), tr!("out.no_installs")),
            Ok(lines) => lines.iter().for_each(|l| println!("{mark} {}: {l}", app.title())),
            Err(e) => println!("{mark} {}: {e:#}", app.title()),
        }
    }
    let update = if ctx.cfg.update_addons {
        let via = chosen.map(|a| tr!("out.via", app = a.title())).unwrap_or_default();
        format!("{}{via}", ctx.cfg.update_text())
    } else {
        tr!("out.off").into()
    };
    let last = updater::last_run().map(|t| tr!("out.last", time = t)).unwrap_or_default();
    println!("{}", tr!("out.updates", update = update, last = last));
    Ok(())
}

fn cmd_backup(ctx: &Ctx, flavor: Option<&str>, kind: Option<Kind>, force: bool) -> Result<()> {
    let report = app::backup_all(ctx, flavor, kind, force, false)?;
    if !report.errors.is_empty() {
        bail!(tr!("out.some_failed"));
    }
    Ok(())
}

fn cmd_list(ctx: &Ctx, flavor: Option<&str>, kind: Option<Kind>) -> Result<()> {
    let store = ctx.store();
    for fl in ctx.flavors(flavor)? {
        for k in kind.map_or(Kind::ALL.to_vec(), |k| vec![k]) {
            let list = store.list(&fl, k)?;
            println!("{}", tr!("out.list_header", flavor = fl.name, what = k.source_rel(), n = list.len()));
            for s in list.iter().rev() {
                println!("  {:<28} {:>10}", s.id, snapshot::human_size(s.size));
            }
        }
    }
    Ok(())
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_lowercase();
    Ok(answer == "y" || answer == "yes" || tr!("out.yes_words").split(' ').any(|w| w == answer))
}

fn cmd_restore(ctx: &Ctx, spec: Option<&str>, flavor: Option<&str>, kind: Option<Kind>, yes: bool) -> Result<()> {
    let (fl, plan) = app::restore_plan(ctx, flavor, spec, kind)?;
    app::ensure_game_closed(&fl)?;
    for (k, s) in &plan {
        println!("{} {} ← {}", fl.name, k.source_rel(), s.id);
    }
    if !yes && !confirm(tr!("out.confirm_restore"))? {
        return Ok(());
    }
    app::restore_apply(ctx, &fl, &plan)
}

fn cmd_chars(ctx: &Ctx, flavor: Option<&str>) -> Result<()> {
    for fl in ctx.flavors(flavor)? {
        let list = chars::list(&fl.wtf_dir());
        println!("{}", tr!("out.chars_header", flavor = fl.name, n = list.len()));
        for c in list {
            println!("  {c}");
        }
    }
    Ok(())
}

fn cmd_copy_char(ctx: &Ctx, from: &str, to: &[String], parts: &[chars::Part], flavor: Option<&str>, dry_run: bool) -> Result<()> {
    let fl = ctx.single_flavor(flavor)?;
    if !dry_run {
        app::ensure_game_closed(&fl)?;
    }
    let list = chars::list(&fl.wtf_dir());
    let src = chars::find(&list, from)?;
    let targets = to.iter().map(|t| chars::find(&list, t)).collect::<Result<Vec<_>>>()?;
    let parts = if parts.is_empty() { &chars::Part::ALL[..] } else { parts };

    if !dry_run && let Outcome::Created(pre) = ctx.store().backup(&fl, Kind::Wtf, true, Some("pre-copy"))? {
        log::info!("{}", tr!("out.pre_copy_saved", id = pre.id));
    }
    for dst in targets {
        let done = chars::copy(src, dst, parts, dry_run)?;
        let line = if dry_run {
            tr!("out.will_copy", n = done.len())
        } else {
            tr!("out.copied", n = done.len())
        };
        println!("{src} → {dst}: {line}");
        for f in done {
            println!("  {f}");
        }
    }
    Ok(())
}

fn cmd_watch(ctx: &Ctx, interval: Option<u64>) -> Result<()> {
    match interval {
        Some(m) => log::info!("{}", tr!("out.watch_interval", n = m)),
        None => log::info!("watch: {}", ctx.cfg.schedule_text().to_lowercase()),
    }
    // Nobody sends requests here; the sender only keeps the loop alive.
    let (_tx, rx) = std::sync::mpsc::channel();
    watch::run(&ctx.config_path, ctx.explicit_config.as_deref(), interval, &rx, |event| {
        if let watch::Event::Backup { result: Err(e), .. } = event {
            log::error!("{e:#}");
        }
    });
    Ok(())
}

fn cmd_schedule(ctx: &Ctx, action: ScheduleCmd) -> Result<()> {
    let cfg_arg = ctx.explicit_config.as_deref();
    match action {
        ScheduleCmd::Interval { every } => {
            let every = every.unwrap_or(24 * 60);
            schedule::install_interval(every, cfg_arg)?;
            println!("{}", tr!("out.task_added", n = every));
        }
        ScheduleCmd::Logon { headless } => {
            schedule::install_logon(cfg_arg, headless)?;
            let what = if headless { "watch" } else { "tray" };
            println!("{}", tr!("out.logon_added", what = what));
        }
        ScheduleCmd::Remove => {
            let a = schedule::remove_interval()?;
            let b = schedule::remove_logon()?;
            println!("{}", if a || b { tr!("out.removed") } else { tr!("out.nothing_set") });
        }
        ScheduleCmd::Status => {
            let task = if schedule::interval_installed() {
                tr!("out.task_present")
            } else {
                tr!("out.none")
            };
            println!("{:<13}{task}", tr!("out.scheduler"));
            match schedule::logon_command() {
                Some(cmd) => println!("{:<13}{cmd}", tr!("out.autostart")),
                None => println!("{:<13}{}", tr!("out.autostart"), tr!("out.none")),
            }
            println!("{:<13}{}", tr!("out.log"), config::default_log_path().display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every visible command and argument has its `--help` text in the locales.
    #[test]
    fn every_command_and_argument_has_help_text() {
        fn walk(cmd: &clap::Command, path: &str, missing: &mut Vec<String>) {
            let key = |k: &str| if path.is_empty() { format!("cli.{k}") } else { format!("cli.{path}.{k}") };
            if !path.is_empty() && i18n::lookup(&key("about")).is_none() {
                missing.push(key("about"));
            }
            for arg in cmd.get_arguments().filter(|a| !a.is_hide_set()) {
                let id = arg.get_id().as_str();
                if id != "help" && id != "version" && i18n::lookup(&key(id)).is_none() {
                    missing.push(key(id));
                }
            }
            for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
                let name = sub.get_name();
                walk(sub, &if path.is_empty() { name.to_string() } else { format!("{path}.{name}") }, missing);
            }
        }
        let mut missing = Vec::new();
        walk(&Cli::command(), "", &mut missing);
        assert!(missing.is_empty(), "no --help text for {missing:?}");
        // And the localized command still parses.
        localize(Cli::command(), "").debug_assert();
    }
}
