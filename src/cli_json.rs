//! `--json`: one JSON object on stdout for a command, errors included (`docs/cli-json.md`). The text commands of
//! `main.rs` are not touched; this is a thin layer over the same core calls. Nothing here is localized: the only
//! free text is `error.detail`, the English error chain, for logs.

use crate::{Cli, Cmd, ConfigCmd, ProfileCmd};
use anyhow::Result;
use chrono::{Local, NaiveDateTime, SecondsFormat, TimeZone};
use serde_json::{Value, json};
use std::io::Write;
use wow_tools_core::app::{self, Ctx};
use wow_tools_core::errors::{self, Coded};
use wow_tools_core::i18n;
use wow_tools_core::snapshot::{Kind, Snapshot};
use wow_tools_core::wow::Flavor;
use wow_tools_core::{chars, config, macro_names, macros, maintenance, profiles, updater, wow};

/// The envelope version; see "Versioning" in `docs/cli-json.md`.
const V: u32 = 1;

/// A failed command, with the data of the partly done run when there is some.
pub struct Failure {
    err: anyhow::Error,
    data: Option<Value>,
}

impl From<anyhow::Error> for Failure {
    fn from(err: anyhow::Error) -> Self {
        Failure { err, data: None }
    }
}

type Res = std::result::Result<Value, Failure>;

/// Runs the command of `cli`, prints the object, returns the exit code.
pub fn main(cli: Cli) -> i32 {
    let (value, code) = execute(cli);
    print(&value);
    code
}

/// A command line that clap refused: the same envelope, exit code 2.
pub fn invalid_args(e: &clap::Error) -> i32 {
    let (value, code) = invalid_args_envelope(e);
    print(&value);
    code
}

fn invalid_args_envelope(e: &clap::Error) -> (Value, i32) {
    let coded = Coded::invalid_args(&format!("{:?}", e.kind()), e.render().to_string());
    envelope(None, Err(anyhow::Error::new(coded).into()))
}

fn print(value: &Value) {
    // A closed pipe is the reader's business.
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn execute(cli: Cli) -> (Value, i32) {
    let name = command_name(cli.cmd.as_ref());
    let result = run(cli);
    envelope(Some(&name), result)
}

fn envelope(command: Option<&str>, result: Res) -> (Value, i32) {
    match result {
        Ok(data) => (json!({ "v": V, "type": "result", "command": command, "data": data }), 0),
        Err(Failure { err, data }) => {
            let (code, params) = errors::classify(&err);
            let mut value = json!({
                "v": V,
                "type": "error",
                "command": command,
                "error": { "code": code, "params": params, "detail": format!("{err:#}") },
            });
            if let Some(data) = data {
                value["data"] = data;
            }
            let exit = if matches!(code, errors::INVALID_ARGS | errors::CONFIRMATION_REQUIRED | errors::JSON_UNSUPPORTED) {
                2
            } else {
                1
            };
            (value, exit)
        }
    }
}

fn command_name(cmd: Option<&Cmd>) -> String {
    match cmd {
        None => "tray",
        Some(Cmd::Tray { .. }) => "tray",
        Some(Cmd::Init { .. }) => "init",
        Some(Cmd::Status) => "status",
        Some(Cmd::Backup { .. }) => "backup",
        Some(Cmd::List { .. }) => "list",
        Some(Cmd::Restore { .. }) => "restore",
        Some(Cmd::Chars { .. }) => "chars",
        Some(Cmd::Macros { .. }) => "macros",
        Some(Cmd::CopyChar { .. }) => "copy-char",
        Some(Cmd::Watch { .. }) => "watch",
        Some(Cmd::UpdateAddons) => "update-addons",
        Some(Cmd::Clean { .. }) => "clean",
        Some(Cmd::Outdated) => "outdated",
        Some(Cmd::SelfUpdate { .. }) => "self-update",
        Some(Cmd::Profile { action }) => match action {
            ProfileCmd::Save { .. } => "profile save",
            ProfileCmd::List => "profile list",
            ProfileCmd::Apply { .. } => "profile apply",
            ProfileCmd::Delete { .. } => "profile delete",
        },
        Some(Cmd::Config { action }) => match action {
            ConfigCmd::Get { .. } => "config get",
            ConfigCmd::Set { .. } => "config set",
            ConfigCmd::Unset { .. } => "config unset",
        },
        Some(Cmd::Schedule { .. }) => "schedule",
        Some(Cmd::Quit) => "quit",
        Some(Cmd::UiSnapshot { .. }) => "ui-snapshot",
    }
    .to_string()
}

fn run(cli: Cli) -> Res {
    let name = command_name(cli.cmd.as_ref());
    let cmd = match cli.cmd {
        Some(Cmd::Tray { .. } | Cmd::Watch { .. } | Cmd::SelfUpdate { .. } | Cmd::Schedule { .. } | Cmd::Quit | Cmd::UiSnapshot { .. }) | None => {
            return Err(anyhow::Error::new(Coded::json_unsupported(&name, tr!("out.json_unsupported", command = name))).into());
        }
        Some(cmd) => cmd,
    };
    let config_path = cli.config.clone().unwrap_or_else(config::default_config_path);
    let ctx = Ctx::load(&config_path, cli.config.as_deref())?;
    dispatch(&ctx, cmd)
}

fn dispatch(ctx: &Ctx, cmd: Cmd) -> Res {
    Ok(match cmd {
        Cmd::Init { wow_path, backup_dir, force } => init(ctx, wow_path, backup_dir, force)?,
        Cmd::Status => status(ctx)?,
        Cmd::Backup { flavor, kind, force } => return backup(ctx, flavor.as_deref(), kind, force),
        Cmd::List { flavor, kind } => list(ctx, flavor.as_deref(), kind)?,
        Cmd::Restore { snapshot, flavor, kind, yes } => restore(ctx, snapshot.as_deref(), flavor.as_deref(), kind, yes)?,
        Cmd::Chars { flavor } => chars_list(ctx, flavor.as_deref())?,
        Cmd::Macros { flavor, update_names } => return macros_check(ctx, flavor.as_deref(), update_names),
        Cmd::CopyChar {
            from,
            to,
            parts,
            flavor,
            dry_run,
        } => return copy_char(ctx, &from, &to, &parts, flavor.as_deref(), dry_run),
        Cmd::UpdateAddons => return update_addons(ctx),
        Cmd::Clean { items } => return clean(ctx, &items),
        Cmd::Outdated => outdated(ctx)?,
        Cmd::Profile { action } => profile(ctx, action)?,
        Cmd::Config { action } => config_cmd(ctx, action)?,
        Cmd::Tray { .. } | Cmd::Watch { .. } | Cmd::SelfUpdate { .. } | Cmd::Schedule { .. } | Cmd::Quit | Cmd::UiSnapshot { .. } => {
            unreachable!("refused in run")
        }
    })
}

// ---------------------------------------------------------------- pieces

/// RFC 3339 with the local offset: the times of the program are local.
fn ts(t: NaiveDateTime) -> String {
    match Local.from_local_datetime(&t).earliest() {
        Some(d) => d.to_rfc3339_opts(SecondsFormat::Secs, false),
        None => t.format("%Y-%m-%dT%H:%M:%S").to_string(),
    }
}

fn error_object(e: &anyhow::Error) -> Value {
    let (code, params) = errors::classify(e);
    json!({ "code": code, "params": params, "detail": format!("{e:#}") })
}

fn snapshot_json(s: &Snapshot) -> Value {
    // `<YYYYmmdd-HHMMSS>[-label]`
    let label = s.id.get(16..).filter(|l| !l.is_empty());
    json!({ "id": s.id, "label": label, "created": ts(s.created), "size": s.size, "path": s.path.display().to_string() })
}

fn char_json(c: &chars::Character) -> Value {
    json!({ "id": c.to_string(), "account": c.account, "realm": c.realm, "name": c.name, "path": c.dir.display().to_string() })
}

fn app_id(app: updater::AddonApp) -> Value {
    serde_json::to_value(app).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------- commands

fn init(ctx: &Ctx, wow_path: Option<std::path::PathBuf>, backup_dir: Option<std::path::PathBuf>, force: bool) -> Result<Value> {
    if ctx.config_path.exists() && !force {
        return Err(Coded::config_exists(&ctx.config_path, tr!("out.config_exists", path = ctx.config_path.display())).into());
    }
    let mut cfg = config::Config::default();
    let root = match wow_path {
        Some(p) => p,
        None => wow::detect_root().ok_or_else(|| Coded::no_wow(tr!("out.wow_not_found")))?,
    };
    wow::resolve_root(Some(&root))?;
    cfg.wow_path = Some(root);
    if let Some(dir) = backup_dir {
        cfg.backup_dir = dir;
    }
    cfg.save(&ctx.config_path)?;
    Ok(json!({ "config_path": ctx.config_path.display().to_string(), "config": serde_json::to_value(&cfg)? }))
}

fn status(ctx: &Ctx) -> Result<Value> {
    let root = match &ctx.cfg.wow_path {
        Some(p) => wow::resolve_root(Some(p)).map(|r| (Some(r), "config")),
        None => Ok((wow::detect_root(), "detected")),
    };
    let wow_info = match root {
        Ok((root, source)) => {
            let all = wow::all_flavors(root.as_deref());
            let source = match (&root, all.is_empty()) {
                (Some(_), _) => source,
                (None, false) => "battlenet",
                (None, true) => "none",
            };
            let selected: Vec<String> = ctx.flavors(None).map(|l| l.into_iter().map(|f| f.name).collect()).unwrap_or_default();
            let store = ctx.store();
            let mut flavors = Vec::new();
            for fl in &all {
                let mut series = serde_json::Map::new();
                for kind in Kind::ALL {
                    let list = store.list(fl, kind)?;
                    series.insert(
                        kind.series_name().into(),
                        json!({ "count": list.len(), "last": list.last().map(|s| ts(s.created)) }),
                    );
                }
                flavors.push(json!({
                    "name": fl.name,
                    "version": fl.version,
                    "dir": fl.dir.display().to_string(),
                    "selected": selected.contains(&fl.name),
                    "characters": chars::list(&fl.wtf_dir()).len(),
                    "series": series,
                }));
            }
            (json!({ "root": root.map(|r| r.display().to_string()), "source": source }), flavors)
        }
        Err(e) => (json!({ "root": null, "source": "config", "error": error_object(&e) }), Vec::new()),
    };
    let games: Vec<Value> = wow::running_games()
        .into_iter()
        .map(|g| json!({ "name": g.name, "dir": g.dir.map(|d| d.display().to_string()) }))
        .collect();
    let chosen = updater::chosen(ctx.cfg.update_app);
    let apps: Vec<Value> = updater::AddonApp::installed()
        .into_iter()
        .map(|a| json!({ "app": app_id(a), "chosen": Some(a) == chosen }))
        .collect();
    Ok(json!({
        "app": { "version": env!("CARGO_PKG_VERSION") },
        "config": { "path": ctx.config_path.display().to_string(), "exists": ctx.config_path.exists() },
        "backups": { "dir": ctx.cfg.backup_dir.display().to_string(), "keep": ctx.cfg.keep },
        "wow": wow_info.0,
        "flavors": wow_info.1,
        "games": games,
        "addon_apps": apps,
        "addon_updates": { "enabled": ctx.cfg.update_addons, "app": chosen.map(app_id), "last_run": updater::last_run().map(ts) },
    }))
}

fn backup(ctx: &Ctx, flavor: Option<&str>, kind: Option<Kind>, force: bool) -> Res {
    let report = app::backup_all(ctx, flavor, kind, force, false)?;
    let data = json!({
        "created": report.snapshots.iter().map(|(fl, k, s)| json!({
            "flavor": fl, "kind": k.series_name(), "id": s.id, "size": s.size, "created": ts(s.created),
        })).collect::<Vec<_>>(),
        "unchanged": report.unchanged.iter().map(|(fl, k)| json!({ "flavor": fl, "kind": k.series_name() })).collect::<Vec<_>>(),
        "skipped_game": report.skipped_game,
        "failures": report.failures.iter().map(|f| json!({
            "flavor": f.flavor,
            "kind": f.kind.map(Kind::series_name),
            "error": { "code": f.code, "params": f.params, "detail": f.detail },
        })).collect::<Vec<_>>(),
    });
    if report.failures.is_empty() {
        Ok(data)
    } else {
        Err(Failure {
            err: Coded::partial(tr!("out.some_failed")).into(),
            data: Some(data),
        })
    }
}

fn list(ctx: &Ctx, flavor: Option<&str>, kind: Option<Kind>) -> Result<Value> {
    let store = ctx.store();
    let mut series = Vec::new();
    for fl in ctx.flavors(flavor)? {
        for k in kind.map_or(Kind::ALL.to_vec(), |k| vec![k]) {
            let snapshots: Vec<Value> = store.list(&fl, k)?.iter().rev().map(snapshot_json).collect();
            series.push(json!({ "flavor": fl.name, "kind": k.series_name(), "snapshots": snapshots }));
        }
    }
    Ok(json!({ "series": series }))
}

fn restore(ctx: &Ctx, spec: Option<&str>, flavor: Option<&str>, kind: Option<Kind>, yes: bool) -> Result<Value> {
    // Nothing is asked in JSON mode: no `--yes`, no restore, and stdin stays unread.
    if !yes {
        return Err(Coded::confirmation_required(tr!("out.json_needs_yes")).into());
    }
    let (fl, plan) = app::restore_plan(ctx, flavor, spec, kind)?;
    app::ensure_game_closed(&fl)?;
    app::restore_apply(ctx, &fl, &plan)?;
    let plan: Vec<Value> = plan.iter().map(|(k, s)| json!({ "kind": k.series_name(), "snapshot": s.id })).collect();
    Ok(json!({ "flavor": fl.name, "plan": plan, "applied": true }))
}

fn chars_list(ctx: &Ctx, flavor: Option<&str>) -> Result<Value> {
    let flavors: Vec<Value> = ctx
        .flavors(flavor)?
        .into_iter()
        .map(|fl| json!({ "flavor": fl.name, "characters": chars::list(&fl.wtf_dir()).iter().map(char_json).collect::<Vec<_>>() }))
        .collect();
    Ok(json!({ "flavors": flavors }))
}

fn copy_char(ctx: &Ctx, from: &str, to: &[String], parts: &[chars::Part], flavor: Option<&str>, dry_run: bool) -> Res {
    let mut source = String::new();
    let mut targets = Vec::new();
    let done = crate::copy_char(ctx, from, to, parts, flavor, dry_run, |src, dst, files| {
        source = src.to_string();
        targets.push(json!({ "character": dst.to_string(), "files": files }));
    });
    let data = json!({ "dry_run": dry_run, "source": source, "targets": targets });
    match done {
        Ok(()) => Ok(data),
        // The targets done before the error stay in `data`.
        Err(err) => Err(Failure {
            err,
            data: (!targets.is_empty()).then_some(data),
        }),
    }
}

fn update_addons(ctx: &Ctx) -> Res {
    let started = Local::now().naive_local();
    // The history rows are stored for the tray: they are written in the user's language, not in the English of the envelope.
    let json_lang = i18n::current();
    i18n::set(ctx.cfg.language);
    let result = app::update_addons(ctx, true);
    i18n::set(Some(json_lang));
    // The manager and ElvUI each write a row, both with the time the run started.
    let entries: Vec<updater::Entry> = updater::history().into_iter().take_while(|e| e.time >= started).collect();
    let data = update_data(&entries);
    match result {
        Err(err) => Err(Failure {
            err,
            data: (!entries.is_empty()).then_some(data),
        }),
        // ElvUI failed beside a manager that did not.
        Ok(_) if entries.iter().any(|e| !e.ok) => Err(Failure {
            err: Coded::partial(tr!("out.some_failed")).into(),
            data: Some(data),
        }),
        Ok(_) => Ok(data),
    }
}

/// `entries`: the history rows this run wrote, without their text (it is in the language of the user).
/// The lists come from `details` (`null` each for a row without them); `manual` is the run, `by_hand` the addons.
fn update_data(entries: &[updater::Entry]) -> Value {
    let row = |e: &updater::Entry| {
        let d = e.details.as_ref();
        json!({
            "time": ts(e.time), "app": e.app, "ok": e.ok, "manual": e.manual, "skipped": e.skipped,
            "updated": d.map(|d| &d.updated), "failed": d.map(|d| &d.failed), "outdated": d.map(|d| &d.outdated), "by_hand": d.map(|d| &d.by_hand),
            "error": d.map(|d| &d.error),
        })
    };
    json!({ "ran": !entries.is_empty(), "entries": entries.iter().map(row).collect::<Vec<_>>() })
}

fn clean(ctx: &Ctx, items: &[String]) -> Res {
    if items.is_empty() {
        let scan = maintenance::scan(&ctx.flavors(None)?);
        let items: Vec<Value> = maintenance::Item::ALL
            .into_iter()
            .map(|item| {
                let found = scan.get(item);
                json!({ "key": item.key(), "files": found.files, "bytes": found.bytes, "names": found.names })
            })
            .collect();
        let big: Vec<Value> = scan.big.iter().map(|(name, bytes)| json!({ "name": name, "bytes": bytes })).collect();
        return Ok(json!({ "items": items, "big": big }));
    }
    // Every key is checked before anything is cleaned: wrong use (exit 2) means nothing ran.
    let mut chosen = Vec::new();
    for key in items {
        let Some(item) = maintenance::Item::from_key(key) else {
            return Err(anyhow::Error::new(Coded::invalid_args("InvalidValue", key.clone())).into());
        };
        chosen.push(item);
    }
    let mut results = Vec::new();
    for item in chosen {
        let c = match maintenance::clean_item(ctx, item) {
            Ok(c) => c,
            // What the items before it did stays in `data`.
            Err(err) => {
                let data = (!results.is_empty()).then(|| json!({ "results": results }));
                return Err(Failure { err, data });
            }
        };
        let skipped: Vec<&str> = c.skipped.iter().map(|f| f.name.as_str()).collect();
        results.push(json!({ "key": item.key(), "files": c.files, "bytes": c.bytes, "failed": c.failed, "skipped_flavors": skipped }));
    }
    Ok(json!({ "results": results }))
}

fn outdated(ctx: &Ctx) -> Result<Value> {
    let flavors: Vec<Value> = ctx
        .flavors(None)?
        .iter()
        .map(|fl| {
            let judged = fl.version.is_some();
            let addons = if judged { maintenance::outdated(fl) } else { Vec::new() };
            json!({ "name": fl.name, "version": fl.version, "judged": judged, "addons": addons })
        })
        .collect();
    Ok(json!({ "flavors": flavors }))
}

// ---------------------------------------------------------------- profiles

fn profile_json(ctx: &Ctx, p: &profiles::Profile, number: Option<usize>) -> Value {
    let m = &p.meta;
    json!({
        "id": p.id(&profiles::root(ctx)),
        "number": number,
        "flavor": m.flavor,
        "account": m.account,
        "realm": m.realm,
        "name": m.name,
        "created": ts(m.created),
        "files": m.files,
        "addons": m.addons,
    })
}

fn profile(ctx: &Ctx, action: ProfileCmd) -> Result<Value> {
    Ok(match action {
        ProfileCmd::Save { character, flavor } => {
            let fl = ctx.single_flavor(flavor.as_deref())?;
            let list = chars::list(&fl.wtf_dir());
            let saved = profiles::save(ctx, &fl, chars::find(&list, &character)?)?;
            let number = profiles::list(&profiles::root(ctx))
                .iter()
                .position(|p| p.path == saved.profile.path)
                .map(|i| i + 1);
            json!({ "profile": profile_json(ctx, &saved.profile, number), "unchanged": !saved.new })
        }
        ProfileCmd::List => {
            let list = profiles::list(&profiles::root(ctx));
            let profiles: Vec<Value> = list.iter().enumerate().map(|(i, p)| profile_json(ctx, p, Some(i + 1))).collect();
            json!({ "profiles": profiles })
        }
        ProfileCmd::Apply { profile, to } => {
            let profile = crate::pick_profile(ctx, &profile)?;
            let fl = ctx.single_flavor(Some(&profile.meta.flavor))?;
            let list = chars::list(&fl.wtf_dir());
            let target = chars::find(&list, &to)?;
            profiles::apply(ctx, &fl, &profile, target)?;
            json!({ "profile": profile_json(ctx, &profile, None), "character": char_json(target) })
        }
        ProfileCmd::Delete { profile } => {
            let profile = crate::pick_profile(ctx, &profile)?;
            profiles::delete(&profile)?;
            json!({ "deleted": profile.id(&profiles::root(ctx)) })
        }
    })
}

// ---------------------------------------------------------------- config

fn config_cmd(ctx: &Ctx, action: ConfigCmd) -> Result<Value> {
    let defaults = config::Config::default();
    Ok(match action {
        ConfigCmd::Get { key: None } => json!({
            "path": ctx.config_path.display().to_string(),
            "exists": ctx.config_path.exists(),
            "config": ctx.cfg.image(),
            "defaults": defaults.image(),
        }),
        ConfigCmd::Get { key: Some(key) } => json!({ "key": key, "value": ctx.cfg.get_key(&key)?, "default": defaults.get_key(&key)? }),
        ConfigCmd::Set { key, value } => changed(ctx, &key, Some(config::parse_value(&value)))?,
        ConfigCmd::Unset { key } => changed(ctx, &key, None)?,
    })
}

fn changed(ctx: &Ctx, key: &str, value: Option<Value>) -> Result<Value> {
    let (old, new) = crate::config_change(ctx, key, value)?;
    Ok(json!({ "key": key, "value": new, "changed": old != new }))
}

// ---------------------------------------------------------------- macros

fn level_name(l: macros::Level) -> &'static str {
    match l {
        macros::Level::Broken => "broken",
        macros::Level::Warn => "warn",
        macros::Level::Info => "info",
    }
}

/// An alias finding keeps its realms as `Name-Realm (ruRU[, <note>])`: the note is text, so it is cut off
/// and only told apart (`guessed`: the language was taken from the realm's name).
fn alias_target(label: &str) -> Value {
    let split = label.rsplit_once(" (").and_then(|(who, rest)| Some((who, rest.strip_suffix(')')?)));
    match split {
        Some((who, rest)) => match rest.split_once(", ") {
            Some((locale, _)) => json!({ "character": who, "locale": locale, "guessed": true }),
            None => json!({ "character": who, "locale": rest, "guessed": false }),
        },
        None => json!({ "character": label, "locale": null, "guessed": false }),
    }
}

fn problem_json(kind: &macros::Kind) -> Value {
    use macros::Kind as K;
    match kind {
        K::Deprecated { func, replacement, patch, url } => {
            json!({ "type": "deprecated", "func": func, "replacement": replacement, "patch": patch, "url": url })
        }
        K::TooLong { chars } => json!({ "type": "too_long", "chars": chars, "max": macros::MAX_LEN }),
        K::Brackets => json!({ "type": "brackets" }),
        K::Alias {
            alias,
            command,
            locales,
            bad,
            soft,
            firm,
        } => json!({
            "type": "alias", "alias": alias, "command": command, "locales": locales, "firm": firm,
            "bad": bad.iter().map(|l| alias_target(l)).collect::<Vec<_>>(),
            "soft": soft.iter().map(|l| alias_target(l)).collect::<Vec<_>>(),
        }),
        K::Secret { func } => json!({ "type": "secret", "func": func }),
        K::WrongLanguage { name, lang, fix, how } => {
            let how = match how {
                macros::How::Firm => "firm",
                macros::How::Maybe => "maybe",
                macros::How::Client => "client",
                macros::How::Realm => "realm",
            };
            json!({ "type": "wrong_language", "name": name, "lang": lang, "fix": fix, "how": how })
        }
        K::UnknownName { name } => json!({ "type": "unknown_name", "name": name }),
    }
}

fn macros_data(scan: &macros::Scan, flavors: &[Flavor]) -> Value {
    let count = |l| scan.findings.iter().filter(|f| f.level() == l).count();
    let findings: Vec<Value> = scan
        .findings
        .iter()
        .zip(macros::first_fixes(&scan.findings))
        .map(|(f, fix)| {
            let o = &f.owner;
            let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
            json!({
                "level": level_name(f.level()),
                "flavor": o.flavor,
                "scope": {
                    "kind": if f.account { "account" } else { "character" },
                    "account": o.account,
                    "realm": some(&o.realm),
                    "character": some(&o.character),
                },
                "macro": f.name,
                "line": f.line,
                "problem": problem_json(&f.kind),
                "fixed": fix.map(|x| serde_json::to_value(&**x).unwrap_or(Value::Null)),
            })
        })
        .collect();
    // `missing` holds titles; the folder names are what a shell knows.
    let missing: Vec<&str> = scan
        .missing
        .iter()
        .map(|t| flavors.iter().find(|f| &f.title() == t).map_or(t.as_str(), |f| f.name.as_str()))
        .collect();
    json!({
        "checked": scan.checked,
        "missing": missing,
        "levels": { "broken": count(macros::Level::Broken), "warn": count(macros::Level::Warn), "info": count(macros::Level::Info) },
        "findings": findings,
    })
}

fn macros_check(ctx: &Ctx, flavor: Option<&str>, update_names: bool) -> Res {
    let flavors = ctx.flavors(flavor)?;
    // The progress lines go to the log, stdout is for the one object.
    let downloaded = if update_names {
        macro_names::update_for(&flavors, &mut |line| log::info!("{line}"))
    } else {
        Ok(())
    };
    let mut data = macros_data(&macros::scan(&flavors, &macro_names::dir()), &flavors);
    if update_names {
        data["names_updated"] = json!(downloaded.is_ok());
    }
    // The check comes even when the download failed (the cached names still serve), then the error.
    match downloaded {
        Ok(()) => Ok(data),
        Err(err) => Err(Failure { err, data: Some(data) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};
    use std::fs;
    use std::path::{Path, PathBuf};
    use wow_tools_core::{lock, macro_fix};

    /// A fake WoW tree with one flavor and two characters, a scratch config and a scratch backup folder.
    /// The lock files go to one scratch folder for the whole test run (`WOW_TOOLS_LOCKS_DIR`).
    struct Fixture {
        tmp: tempfile::TempDir,
        root: PathBuf,
        config: PathBuf,
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn fixture() -> Fixture {
        static LOCKS: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        LOCKS.get_or_init(|| {
            let dir = tempfile::tempdir().unwrap();
            // Before any lock is taken; only the core built with `test-ru` (our dev-dependency) reads it.
            unsafe { std::env::set_var("WOW_TOOLS_LOCKS_DIR", dir.path()) };
            dir
        });
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wow");
        let fl = root.join("_retail_");
        let name = fl.join("WTF/Account/ACC/Realm/Name");
        put(&name.join("layout-local.txt"), "layout");
        put(
            &name.join("macros-cache.txt"),
            "VER 3 0000000000000001 \"Fire\" \"INV_Misc_QuestionMark\"\n/run print(GetItemCount(1))\nEND\n",
        );
        fs::create_dir_all(fl.join("WTF/Account/ACC/Realm/Other")).unwrap();
        put(
            &fl.join("WTF/Account/ACC/macros-cache.txt"),
            "VER 3 0000000000000002 \"Bag\" \"INV_Misc_QuestionMark\"\n/run print(GetItemCount(2))\nEND\n",
        );
        put(&fl.join("WTF/Account/ACC/SavedVariables/Orphan.lua"), "x = 1");
        put(&fl.join("Interface/AddOns/X/X.toc"), "## Interface: 110000\n");
        put(&fl.join("Interface/AddOns/X/X.lua"), "-- x");
        let config = tmp.path().join("config.toml");
        // `flavors` and no waiting for a running game: the tests do not depend on the machine they run on.
        let cfg = config::Config {
            wow_path: Some(root.clone()),
            backup_dir: tmp.path().join("backups"),
            flavors: vec!["_retail_".into()],
            skip_while_running: false,
            ..config::Config::default()
        };
        cfg.save(&config).unwrap();
        Fixture { tmp, root, config }
    }

    impl Fixture {
        /// Runs `wow-tools --config <scratch> --json <args>`; the object as printed and the exit code.
        fn raw(&self, args: &[&str]) -> (Value, i32) {
            let mut argv = vec!["wow-tools", "--config", self.config.to_str().unwrap(), "--json"];
            argv.extend_from_slice(args);
            let (value, code) = execute(Cli::try_parse_from(argv).unwrap());
            // One line, whatever is in it.
            assert!(!value.to_string().contains('\n'));
            (value, code)
        }

        fn call(&self, args: &[&str]) -> (Value, i32) {
            let (mut value, code) = self.raw(args);
            self.normalize(&mut value);
            (value, code)
        }

        /// The temp root becomes `<root>`, times `<ts>`, the `YYYYmmdd-HHMMSS` of snapshot ids `<id>`.
        fn normalize(&self, value: &mut Value) {
            let root = self.tmp.path().display().to_string();
            match value {
                Value::String(s) => {
                    if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
                        *s = "<ts>".into();
                        return;
                    }
                    let text = s.replace(&root, "<root>").replace(&root.replace('\\', "/"), "<root>").replace('\\', "/");
                    let chars: Vec<char> = text.chars().collect();
                    let stamp = |i: usize| {
                        chars.len() >= i + 15
                            && chars[i..i + 15]
                                .iter()
                                .enumerate()
                                .all(|(k, c)| if k == 8 { *c == '-' } else { c.is_ascii_digit() })
                    };
                    let (mut out, mut i) = (String::new(), 0);
                    while i < chars.len() {
                        if stamp(i) {
                            out.push_str("<id>");
                            i += 15;
                        } else {
                            out.push(chars[i]);
                            i += 1;
                        }
                    }
                    *s = out;
                }
                Value::Array(a) => a.iter_mut().for_each(|v| self.normalize(v)),
                Value::Object(o) => o.values_mut().for_each(|v| self.normalize(v)),
                _ => {}
            }
        }

        /// The `data` of a command that must succeed.
        fn ok(&self, args: &[&str]) -> Value {
            let (v, code) = self.call(args);
            assert_eq!((code, v["type"].as_str(), v["v"].as_u64()), (0, Some("result"), Some(1)), "{v}");
            v["data"].clone()
        }

        fn err(&self, args: &[&str]) -> (Value, i32) {
            let (v, code) = self.call(args);
            assert_eq!((v["type"].as_str(), v["v"].as_u64()), (Some("error"), Some(1)), "{v}");
            (v, code)
        }
    }

    /// The code and params of an error object.
    fn coded(v: &Value) -> (Value, Value) {
        (v["error"]["code"].clone(), v["error"]["params"].clone())
    }

    fn character(name: &str) -> Value {
        json!({ "id": format!("ACC/Realm/{name}"), "account": "ACC", "realm": "Realm", "name": name, "path": format!("<root>/wow/_retail_/WTF/Account/ACC/Realm/{name}") })
    }

    /// Sizes depend on the zip writer: only that they are there.
    fn zero_sizes(value: &mut Value) {
        match value {
            Value::Object(o) => {
                if let Some(size) = o.get_mut("size") {
                    assert!(size.as_u64().unwrap() > 0);
                    *size = json!(0);
                }
                o.values_mut().for_each(zero_sizes);
            }
            Value::Array(a) => a.iter_mut().for_each(zero_sizes),
            _ => {}
        }
    }

    #[test]
    fn status_describes_the_install() {
        let f = fixture();
        let mut data = f.ok(&["status"]);
        // These read the machine the test runs on: only their type is checked, and flavors Battle.net adds are left out.
        assert!(data["games"].is_array() && data["addon_apps"].is_array());
        data["flavors"]
            .as_array_mut()
            .unwrap()
            .retain(|v| v["dir"].as_str().unwrap().starts_with("<root>"));
        assert_eq!(
            data["flavors"],
            json!([{
                "name": "_retail_", "version": null, "dir": "<root>/wow/_retail_", "selected": true, "characters": 2,
                "series": { "wtf": { "count": 0, "last": null }, "addons": { "count": 0, "last": null } },
            }])
        );
        assert_eq!(data["app"], json!({ "version": env!("CARGO_PKG_VERSION") }));
        assert_eq!(data["config"], json!({ "path": "<root>/config.toml", "exists": true }));
        assert_eq!(data["backups"]["dir"], "<root>/backups");
        assert_eq!(data["wow"], json!({ "root": "<root>/wow", "source": "config" }));
        assert_eq!(data["addon_updates"]["enabled"], config::Config::default().update_addons);
        assert!(data["addon_updates"].as_object().unwrap().contains_key("last_run"));
    }

    #[test]
    fn status_of_a_bad_install_says_why_instead_of_failing() {
        let f = fixture();
        let cfg = config::Config {
            wow_path: Some(f.tmp.path().join("nothing")),
            ..config::Config::load(&f.config).unwrap()
        };
        cfg.save(&f.config).unwrap();
        let data = f.ok(&["status"]);
        assert_eq!(data["flavors"], json!([]));
        assert_eq!(
            (data["wow"]["root"].clone(), data["wow"]["error"]["code"].clone()),
            (Value::Null, json!("no_wow"))
        );
    }

    #[test]
    fn init_writes_the_config_and_refuses_to_overwrite() {
        let f = fixture();
        let new = f.tmp.path().join("new.toml");
        let strings: Vec<String> = [new.to_str().unwrap(), f.root.to_str().unwrap(), f.tmp.path().join("b").to_str().unwrap()]
            .map(String::from)
            .to_vec();
        let run = |force: bool| {
            let mut argv = vec![
                "wow-tools",
                "--config",
                &strings[0],
                "--json",
                "init",
                "--wow-path",
                &strings[1],
                "--backup-dir",
                &strings[2],
            ];
            argv.extend(force.then_some("--force"));
            let (mut v, code) = execute(Cli::try_parse_from(argv).unwrap());
            f.normalize(&mut v);
            (v, code)
        };
        let (v, code) = run(false);
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["data"]["config_path"], "<root>/new.toml");
        assert_eq!(v["data"]["config"]["wow_path"], "<root>/wow");
        assert_eq!(v["data"]["config"]["backup_dir"], "<root>/b");
        assert_eq!(config::Config::load(&new).unwrap().wow_path, Some(f.root.clone()));
        let (v, code) = run(false);
        assert_eq!((code, coded(&v)), (1, (json!("config_exists"), json!({ "path": "<root>/new.toml" }))));
        assert_eq!(run(true).1, 0);
    }

    #[test]
    fn backup_creates_then_finds_nothing_changed_and_list_shows_newest_first() {
        let f = fixture();
        let mut data = f.ok(&["backup"]);
        zero_sizes(&mut data);
        let made = |kind: &str| json!({ "flavor": "_retail_", "kind": kind, "id": "<id>", "size": 0, "created": "<ts>" });
        assert_eq!(
            data,
            json!({ "created": [made("wtf"), made("addons")], "unchanged": [], "skipped_game": null, "failures": [] })
        );

        assert_eq!(
            f.ok(&["backup"]),
            json!({ "created": [], "unchanged": [{ "flavor": "_retail_", "kind": "wtf" }, { "flavor": "_retail_", "kind": "addons" }], "skipped_game": null, "failures": [] })
        );
        // Ids are seconds: a forced second snapshot of one series waits for the next one by itself.
        f.ok(&["backup", "--kind", "wtf", "--force"]);
        let mut list = f.ok(&["list", "--kind", "wtf"]);
        zero_sizes(&mut list);
        let snap = json!({ "id": "<id>", "label": null, "created": "<ts>", "size": 0, "path": "<root>/backups/_retail_/wtf/<id>.zip" });
        assert_eq!(list, json!({ "series": [{ "flavor": "_retail_", "kind": "wtf", "snapshots": [snap, snap] }] }));
        // Newest first.
        let (raw, _) = f.raw(&["list", "--kind", "wtf"]);
        let ids: Vec<&str> = raw["data"]["series"][0]["snapshots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert!(ids[0] > ids[1], "{ids:?}");
    }

    #[test]
    fn restore_needs_yes_and_puts_the_snapshot_back() {
        let f = fixture();
        f.ok(&["backup", "--kind", "wtf"]);
        let layout = f.root.join("_retail_/WTF/Account/ACC/Realm/Name/layout-local.txt");
        fs::write(&layout, "changed").unwrap();
        let (v, code) = f.err(&["restore", "--kind", "wtf"]);
        assert_eq!((code, coded(&v).0), (2, json!("confirmation_required")));
        assert_eq!(fs::read_to_string(&layout).unwrap(), "changed", "nothing was touched");
        let data = f.ok(&["restore", "--kind", "wtf", "--yes"]);
        assert_eq!(
            data,
            json!({ "flavor": "_retail_", "plan": [{ "kind": "wtf", "snapshot": "<id>" }], "applied": true })
        );
        assert_eq!(fs::read_to_string(&layout).unwrap(), "layout");
        // The state before the restore was saved.
        assert_eq!(f.ok(&["list", "--kind", "wtf"])["series"][0]["snapshots"][0]["label"], "pre-restore");
    }

    #[test]
    fn chars_lists_characters_with_the_id_copy_char_takes() {
        let f = fixture();
        assert_eq!(
            f.ok(&["chars"]),
            json!({ "flavors": [{ "flavor": "_retail_", "characters": [character("Name"), character("Other")] }] })
        );
    }

    #[test]
    fn copy_char_shows_the_plan_and_copies() {
        let f = fixture();
        let target = f.root.join("_retail_/WTF/Account/ACC/Realm/Other/layout-local.txt");
        let args = ["copy-char", "--from", "realm/name", "--to", "Realm/Other", "--parts", "layout"];
        let plan = json!({ "dry_run": true, "source": "ACC/Realm/Name", "targets": [{ "character": "ACC/Realm/Other", "files": ["layout-local.txt"] }] });
        assert_eq!(f.ok(&[&args[..], &["--dry-run"]].concat()), plan);
        assert!(!target.exists());
        assert_eq!(f.ok(&args), json!({ "dry_run": false, "source": plan["source"], "targets": plan["targets"] }));
        assert_eq!(fs::read_to_string(&target).unwrap(), "layout");
    }

    #[test]
    fn macros_findings_have_no_text_in_them() {
        let f = fixture();
        let mut data = f.ok(&["macros"]);
        // `fixed` depends on the name tables this machine has cached.
        data["findings"].as_array_mut().unwrap().iter_mut().for_each(|x| x["fixed"] = Value::Null);
        let problem = json!({ "type": "deprecated", "func": "GetItemCount", "replacement": "C_Item.GetItemCount", "patch": "12.1.5", "url": "https://warcraft.wiki.gg/wiki/Patch_12.1.5/API_changes" });
        assert_eq!(
            data,
            json!({
                "checked": 2,
                "missing": [],
                "levels": { "broken": 2, "warn": 0, "info": 0 },
                "findings": [
                    { "level": "broken", "flavor": "_retail_", "scope": { "kind": "account", "account": "ACC", "realm": null, "character": null },
                      "macro": "Bag", "line": 1, "problem": problem, "fixed": null },
                    { "level": "broken", "flavor": "_retail_", "scope": { "kind": "character", "account": "ACC", "realm": "Realm", "character": "Name" },
                      "macro": "Fire", "line": 1, "problem": problem, "fixed": null },
                ],
            })
        );
    }

    #[test]
    fn macro_data_covers_every_problem_and_the_fixed_text() {
        use macros::{Finding, How, Kind as K, Owner};
        let owner = Owner {
            flavor: "_classic_".into(),
            account: "A#1".into(),
            realm: "Tarren Mill".into(),
            character: "Gronckk".into(),
        };
        let at = |kind| Finding {
            flavor: "Classic".into(),
            // Text in the interface language, which no JSON field may carry.
            scope: "Аккаунт".into(),
            account: false,
            name: "M".into(),
            line: 3,
            kind,
            owner: owner.clone(),
            fixed: None,
        };
        let fix = std::sync::Arc::new(macro_fix::MacroFix {
            variants: vec![macro_fix::Variant {
                lang: "enUS".into(),
                text: "/cast Fireball".into(),
                chars: 14,
                over: false,
                rest: 1,
                marks: vec![std::ops::Range { start: 6, end: 14 }],
                check: true,
            }],
        });
        let kinds = vec![
            K::TooLong { chars: 300 },
            K::Brackets,
            K::Secret { func: "UnitHealth".into() },
            K::UnknownName { name: "Heroik".into() },
            K::WrongLanguage {
                name: "Feuerball".into(),
                lang: "deDE".into(),
                fix: "Fireball".into(),
                how: How::Maybe,
            },
            K::Alias {
                alias: "/zauber".into(),
                command: "/cast".into(),
                locales: "deDE".into(),
                bad: vec!["Aaa-Нигде (ruRU, язык по названию сервера)".into()],
                soft: vec!["Bbb-Draenor (deDE)".into()],
                firm: false,
            },
        ];
        let mut findings: Vec<Finding> = kinds.into_iter().map(at).collect();
        findings[0].fixed = Some(fix.clone());
        findings[1].fixed = Some(fix);
        let scan = macros::Scan {
            findings,
            checked: 9,
            missing: vec!["Classic".into(), "Unknown title".into()],
            ..Default::default()
        };
        let data = macros_data(&scan, &[Flavor::new("_classic_", "x")]);
        assert_eq!(data["missing"], json!(["_classic_", "Unknown title"]));
        let f = &data["findings"];
        assert_eq!(f[0]["problem"], json!({ "type": "too_long", "chars": 300, "max": 255 }));
        assert_eq!(
            f[0]["fixed"]["variants"][0],
            json!({ "lang": "enUS", "text": "/cast Fireball", "chars": 14, "over": false, "rest": 1, "marks": [{ "start": 6, "end": 14 }], "check": true })
        );
        // Findings of one macro share a fix; only the first carries it (as in the text report).
        assert_eq!(f[1]["fixed"], Value::Null);
        assert_eq!(f[2]["problem"], json!({ "type": "secret", "func": "UnitHealth" }));
        assert_eq!(
            f[4]["problem"],
            json!({ "type": "wrong_language", "name": "Feuerball", "lang": "deDE", "fix": "Fireball", "how": "maybe" })
        );
        assert_eq!(
            f[5]["problem"],
            json!({
                "type": "alias", "alias": "/zauber", "command": "/cast", "locales": "deDE", "firm": false,
                "bad": [{ "character": "Aaa-Нигде", "locale": "ruRU", "guessed": true }],
                "soft": [{ "character": "Bbb-Draenor", "locale": "deDE", "guessed": false }],
            })
        );
        assert_eq!(
            f[0]["scope"],
            json!({ "kind": "character", "account": "A#1", "realm": "Tarren Mill", "character": "Gronckk" })
        );
        assert_eq!(f[0]["flavor"], "_classic_");
        // Nothing of what the language sets is in the object.
        assert!(!data.to_string().contains("Аккаунт"));
    }

    #[test]
    fn clean_scans_then_removes_one_item() {
        let f = fixture();
        let item = |key: &str, files: u64, bytes: u64, names: &[&str]| json!({ "key": key, "files": files, "bytes": bytes, "names": names });
        assert_eq!(
            f.ok(&["clean"]),
            json!({
                "items": [item("orphans", 1, 5, &["Orphan"]), item("backups", 0, 0, &[]), item("logs", 0, 0, &[]), item("screenshots", 0, 0, &[]), item("cache", 0, 0, &[])],
                "big": [],
            })
        );
        assert_eq!(
            f.ok(&["clean", "orphans"]),
            json!({ "results": [{ "key": "orphans", "files": 1, "bytes": 5, "failed": 0, "skipped_flavors": [] }] })
        );
        assert!(!f.root.join("_retail_/WTF/Account/ACC/SavedVariables/Orphan.lua").exists());
        assert_eq!(f.ok(&["clean", "orphans"])["results"][0]["files"], 0);
    }

    #[test]
    fn config_get_set_unset_through_the_file() {
        let f = fixture();
        let defaults = config::Config::default();
        let data = f.ok(&["config", "get"]);
        assert_eq!((data["path"].clone(), data["exists"].clone()), (json!("<root>/config.toml"), json!(true)));
        // `backup_dir` is a machine path; the rest of the defaults are as they are.
        let mut want = json!(defaults.image());
        f.normalize(&mut want);
        assert_eq!(data["defaults"], want);
        assert_eq!(data["config"]["flavors"], json!(["_retail_"]));
        assert_eq!(data["config"]["update_app"], Value::Null);
        assert_eq!(f.ok(&["config", "get", "keep"]), json!({ "key": "keep", "value": 30, "default": 30 }));

        assert_eq!(f.ok(&["config", "set", "keep", "10"]), json!({ "key": "keep", "value": 10, "changed": true }));
        assert_eq!(f.ok(&["config", "set", "keep", "10"])["changed"], false);
        assert_eq!(f.ok(&["config", "set", "backup_time", "07:30"])["value"], "07:30");
        assert_eq!(f.ok(&["config", "set", "backup_days", r#"["mon","fri"]"#])["value"], json!(["mon", "fri"]));
        f.ok(&["config", "set", "update_app", "curseforge"]);
        let cfg = config::Config::load(&f.config).unwrap();
        assert_eq!((cfg.keep, cfg.backup_time.as_str(), cfg.backup_days.len()), (10, "07:30", 2));
        assert!(cfg.update_app.is_some() && !cfg.skip_while_running, "other keys stay as they were");

        assert_eq!(
            f.ok(&["config", "unset", "update_app"]),
            json!({ "key": "update_app", "value": null, "changed": true })
        );
        assert!(config::Config::load(&f.config).unwrap().update_app.is_none());
    }

    #[test]
    fn config_errors_leave_the_file_alone() {
        let f = fixture();
        let before = fs::read(&f.config).unwrap();
        for (args, code, params) in [
            (
                &["config", "set", "keep", "0"][..],
                2,
                json!({ "kind": "invalid_value", "key": "keep", "reason": "range" }),
            ),
            (
                &["config", "set", "keep", "lots"],
                2,
                json!({ "kind": "invalid_value", "key": "keep", "reason": "type" }),
            ),
            (
                &["config", "set", "backup_time", "25:00"],
                2,
                json!({ "kind": "invalid_value", "key": "backup_time", "reason": "format" }),
            ),
            (&["config", "set", "nope", "1"], 1, json!({ "what": "config_key", "key": "nope" })),
            (&["config", "get", "nope"], 1, json!({ "what": "config_key", "key": "nope" })),
            (&["config", "unset", "nope"], 1, json!({ "what": "config_key", "key": "nope" })),
        ] {
            let (v, c) = f.err(args);
            assert_eq!((c, v["error"]["params"].clone()), (code, params), "{args:?}");
            assert!(v["command"].as_str().unwrap().starts_with("config "));
            assert_eq!(fs::read(&f.config).unwrap(), before, "{args:?}");
        }
        assert_eq!(f.err(&["config", "set", "keep", "0"]).0["error"]["code"], "invalid_args");
        assert_eq!(f.err(&["config", "get", "nope"]).0["error"]["code"], "not_found");
    }

    #[test]
    fn config_set_creates_a_missing_file_from_the_defaults() {
        let f = fixture();
        fs::remove_file(&f.config).unwrap();
        assert_eq!(f.ok(&["config", "get"])["exists"], false);
        f.ok(&["config", "set", "keep", "3"]);
        let expect = config::Config {
            keep: 3,
            ..config::Config::default()
        };
        assert_eq!(config::Config::load(&f.config).unwrap().image(), expect.image());
    }

    /// The console form: `get` prints the file as `init` does, one value or the new line for `set`.
    #[test]
    fn config_text_output_is_the_file_text() {
        let f = fixture();
        let cfg = config::Config::load(&f.config).unwrap();
        let ctx = Ctx::load(&f.config, Some(&f.config)).unwrap();
        assert!(toml::to_string_pretty(&cfg).unwrap().contains("keep = 30"));
        assert_eq!(crate::value_text(&json!("game_exit")), "game_exit");
        assert_eq!(crate::value_text(&json!(["mon"])), r#"["mon"]"#);
        let (old, new) = crate::config_change(&ctx, "keep", Some(json!(4))).unwrap();
        assert_eq!((old, new), (json!(30), json!(4)));
        assert_eq!(config::Config::load(&f.config).unwrap().keep, 4);
    }

    #[test]
    fn outdated_says_when_a_flavor_cannot_be_judged() {
        let f = fixture();
        assert_eq!(
            f.ok(&["outdated"]),
            json!({ "flavors": [{ "name": "_retail_", "version": null, "judged": false, "addons": [] }] })
        );
    }

    #[test]
    fn profiles_save_list_apply_delete_by_number_or_id() {
        let f = fixture();
        assert_eq!(f.ok(&["profile", "list"]), json!({ "profiles": [] }));
        let profile = json!({
            "id": "_retail_/<id> Name - Realm.zip", "number": 1, "flavor": "_retail_", "account": "ACC", "realm": "Realm", "name": "Name",
            "created": "<ts>", "files": 1, "addons": [],
        });
        assert_eq!(f.ok(&["profile", "save", "Realm/Name"]), json!({ "profile": profile, "unchanged": false }));
        // The same character again: nothing new.
        assert_eq!(f.ok(&["profile", "save", "Realm/Name"]), json!({ "profile": profile, "unchanged": true }));
        assert_eq!(f.ok(&["profile", "list"]), json!({ "profiles": [profile] }));

        // The id as printed, with its time in it.
        let id = f.raw(&["profile", "list"]).0["data"]["profiles"][0]["id"].as_str().unwrap().to_string();
        let applied = f.ok(&["profile", "apply", &id, "--to", "Realm/Other"]);
        let mut unnumbered = profile.clone();
        unnumbered["number"] = Value::Null;
        assert_eq!(applied, json!({ "profile": unnumbered, "character": character("Other") }));
        assert_eq!(
            fs::read_to_string(f.root.join("_retail_/WTF/Account/ACC/Realm/Other/layout-local.txt")).unwrap(),
            "layout"
        );
        // The same character is refused.
        let (v, code) = f.err(&["profile", "apply", "1", "--to", "Realm/Name"]);
        assert_eq!((code, coded(&v).0), (1, json!("failed")));

        assert_eq!(f.ok(&["profile", "delete", "1"]), json!({ "deleted": "_retail_/<id> Name - Realm.zip" }));
        let (v, code) = f.err(&["profile", "delete", &id]);
        assert_eq!((code, coded(&v).0), (1, json!("not_found")));
        assert_eq!(v["error"]["params"]["what"], "profile");
    }

    #[test]
    fn update_data_lists_every_row_of_the_run_without_text() {
        assert_eq!(update_data(&[]), json!({ "ran": false, "entries": [] }));
        let time = chrono::NaiveDate::from_ymd_opt(2026, 11, 27).unwrap().and_hms_opt(10, 0, 0).unwrap();
        let row = |app: &str, ok| updater::Entry {
            time,
            app: app.into(),
            ok,
            text: "Обновлено: X".into(),
            manual: true,
            skipped: false,
            details: None,
        };
        let data = update_data(&[row("ElvUI", false), row("CurseForge", true)]);
        assert_eq!(data["ran"], true);
        assert!(data["entries"][0]["time"].as_str().unwrap().starts_with("2026-11-27T10:00:00"));
        assert_eq!(data["entries"][0]["app"], "ElvUI");
        assert_eq!(
            (data["entries"][0]["ok"].clone(), data["entries"][1]["ok"].clone()),
            (json!(false), json!(true))
        );
        assert!(!data.to_string().contains("Обновлено"));
        // A row without details says "unknown" (null), not "none" ([]).
        assert_eq!(
            data["entries"][0],
            json!({ "time": data["entries"][0]["time"], "app": "ElvUI", "ok": false, "manual": true, "skipped": false,
                    "updated": null, "failed": null, "outdated": null, "by_hand": null, "error": null })
        );
    }

    #[test]
    fn update_data_carries_the_details_as_codes_and_data() {
        let time = chrono::NaiveDate::from_ymd_opt(2026, 11, 27).unwrap().and_hms_opt(10, 0, 0).unwrap();
        let change = |name: &str, kind| updater::Change {
            name: name.into(),
            kind,
            from: Some("1.0".into()),
            to: None,
            flavor: Some("_retail_".into()),
        };
        let row = |app: &str, ok, skipped, details| updater::Entry {
            time,
            app: app.into(),
            ok,
            text: "Обновлено 1: BigWigs".into(),
            manual: false,
            skipped,
            details: Some(details),
        };
        let done = updater::RunDetails {
            updated: vec![
                change("BigWigs", updater::ChangeKind::Addon),
                change("WagoUI_Storage", updater::ChangeKind::UiPacks),
            ],
            failed: vec![updater::Failed {
                name: "BugSack".into(),
                from: Some("v1".into()),
                to: Some("v2".into()),
                flavor: None,
                reason: updater::FailReason::NoToken,
            }],
            outdated: vec!["Plater".into()],
            by_hand: vec!["WeakAuras".into()],
            error: None,
        };
        let skipped = updater::RunDetails::of_error(&updater::NotRun("Русский текст".into()).into());
        let data = update_data(&[row("WowUp", true, false, done), row("CurseForge", false, true, skipped)]);
        assert_eq!(
            data["entries"][0],
            json!({
                "time": data["entries"][0]["time"], "app": "WowUp", "ok": true, "manual": false, "skipped": false,
                "updated": [
                    { "name": "BigWigs", "kind": "addon", "from": "1.0", "to": null, "flavor": "_retail_" },
                    { "name": "WagoUI_Storage", "kind": "ui_packs", "from": "1.0", "to": null, "flavor": "_retail_" }
                ],
                "failed": [{ "name": "BugSack", "from": "v1", "to": "v2", "flavor": null, "reason": "no_token" }],
                "outdated": ["Plater"], "by_hand": ["WeakAuras"], "error": null
            })
        );
        assert_eq!(data["entries"][1]["error"], json!({ "code": "not_run", "params": {} }));
        assert_eq!(data["entries"][1]["updated"], json!([]));
        assert!(!data.to_string().contains("Русский"));
    }

    #[test]
    fn copy_char_keeps_the_targets_done_before_the_error() {
        let f = fixture();
        // The second target is the source itself: the first one was copied.
        let (v, code) = f.err(&[
            "copy-char",
            "--from",
            "Realm/Name",
            "--to",
            "Realm/Other",
            "--to",
            "Realm/Name",
            "--parts",
            "layout",
        ]);
        assert_eq!((code, coded(&v).0), (1, json!("failed")));
        assert_eq!(
            v["data"],
            json!({ "dry_run": false, "source": "ACC/Realm/Name", "targets": [{ "character": "ACC/Realm/Other", "files": ["layout-local.txt"] }] })
        );
    }

    #[test]
    fn a_held_lock_is_a_busy_error_with_the_holder() {
        let f = fixture();
        let fl = Flavor::new("_retail_", f.root.join("_retail_"));
        let busy = |operation: &str| {
            (
                json!("busy"),
                json!({ "flavor": "_retail_", "pid": std::process::id(), "operation": operation }),
            )
        };
        let held = lock::acquire(&fl, "restore").unwrap();

        // backup: a run that failed in part, with what it did in `data`.
        let (v, code) = f.err(&["backup"]);
        assert_eq!((code, coded(&v).0), (1, json!("partial")));
        assert_eq!(v["data"]["created"], json!([]));
        let failure = &v["data"]["failures"][0];
        assert_eq!((failure["flavor"].clone(), failure["kind"].clone()), (json!("_retail_"), Value::Null));
        assert_eq!((failure["error"]["code"].clone(), failure["error"]["params"].clone()), busy("restore"));

        for args in [&["clean", "orphans"][..], &["copy-char", "--from", "Realm/Name", "--to", "Realm/Other"]] {
            let (v, code) = f.err(args);
            assert_eq!((code, coded(&v)), (1, busy("restore")), "{args:?}");
        }
        drop(held);

        f.ok(&["backup", "--kind", "wtf"]);
        let held = lock::acquire(&fl, "backup").unwrap();
        let (v, _) = f.err(&["restore", "--yes", "--kind", "wtf"]);
        assert_eq!(coded(&v), busy("backup"));
        drop(held);
    }

    #[test]
    fn lookups_that_find_nothing_are_not_found() {
        let f = fixture();
        let (v, code) = f.err(&["restore", "--yes", "--kind", "wtf", "19990101"]);
        assert_eq!((code, coded(&v)), (1, (json!("not_found"), json!({ "what": "snapshot", "key": "19990101" }))));
        let (v, _) = f.err(&["copy-char", "--from", "Realm/Nobody", "--to", "Realm/Name", "--dry-run"]);
        assert_eq!(coded(&v), (json!("not_found"), json!({ "what": "character", "key": "Realm/Nobody" })));
        let (v, _) = f.err(&["list", "--flavor", "_nothing_"]);
        assert_eq!(coded(&v), (json!("not_found"), json!({ "what": "flavor", "key": "_nothing_" })));
        assert!(v["error"]["detail"].as_str().is_some());
    }

    #[test]
    fn a_broken_config_and_a_missing_install_have_their_codes() {
        let f = fixture();
        fs::write(&f.config, "keep = [").unwrap();
        let (v, code) = f.err(&["status"]);
        assert_eq!((code, coded(&v)), (1, (json!("config_invalid"), json!({ "path": "<root>/config.toml" }))));
        let cfg = config::Config {
            wow_path: Some(f.tmp.path().join("nothing")),
            ..config::Config::default()
        };
        cfg.save(&f.config).unwrap();
        let (v, _) = f.err(&["list"]);
        assert_eq!(coded(&v), (json!("no_wow"), json!({})));
    }

    #[test]
    fn commands_without_json_and_bad_command_lines_are_refused_as_objects() {
        let f = fixture();
        for args in [&["tray"][..], &["watch"], &["self-update", "--check"], &["schedule", "status"], &["quit"], &[]] {
            let (v, code) = f.err(args);
            assert_eq!((code, coded(&v).0), (2, json!("json_unsupported")), "{args:?}");
        }
        assert_eq!(coded(&f.err(&["schedule", "status"]).0).1, json!({ "command": "schedule" }));
        let e = Cli::command().try_get_matches_from(["wow-tools", "--json", "restore", "--bogus"]).unwrap_err();
        let (v, code) = invalid_args_envelope(&e);
        assert_eq!(
            (code, coded(&v), v["command"].clone()),
            (2, (json!("invalid_args"), json!({ "kind": "unknown_argument" })), Value::Null)
        );
        assert!(v["error"]["detail"].as_str().unwrap().contains("--bogus"));
    }

    /// Every object is one line with `v` and `type`, and while the interface language is Russian (as in these tests)
    /// no Cyrillic reaches it: `detail` is the one free text field and `main` forces it to English.
    #[test]
    fn every_object_has_v_and_type_and_no_localized_text() {
        assert_eq!(wow_tools_core::i18n::current(), wow_tools_core::i18n::Lang::Ru);
        fn cyrillic(v: &Value) -> bool {
            match v {
                Value::String(s) => s.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c)),
                Value::Array(a) => a.iter().any(cyrillic),
                Value::Object(o) => o.iter().any(|(k, v)| k != "detail" && cyrillic(v)),
                _ => false,
            }
        }
        let f = fixture();
        f.ok(&["backup", "--kind", "wtf"]);
        let runs: &[&[&str]] = &[
            &["status"],
            &["backup"],
            &["list"],
            &["chars"],
            &["macros"],
            &["clean"],
            &["clean", "orphans"],
            &["outdated"],
            &["profile", "list"],
            &["restore", "--kind", "wtf"],
            &["restore", "--yes", "--kind", "wtf", "19990101"],
            &["list", "--flavor", "_nothing_"],
            &["copy-char", "--from", "Realm/Nobody", "--to", "Realm/Name", "--dry-run"],
            &["tray"],
        ];
        for args in runs {
            let (v, _) = f.raw(args);
            assert_eq!(v["v"], 1, "{args:?}");
            assert!(matches!(v["type"].as_str(), Some("result" | "error")), "{args:?}");
            assert!(!cyrillic(&v), "{args:?}: {v}");
        }
    }

    #[test]
    fn the_envelope_is_one_line_with_the_version() {
        let (ok, code) = envelope(Some("status"), Ok(json!({ "text": "a\nb" })));
        assert_eq!(
            (code, ok.to_string()),
            (0, r#"{"v":1,"type":"result","command":"status","data":{"text":"a\nb"}}"#.to_string())
        );
        let (err, code) = envelope(Some("x"), Err(anyhow::anyhow!("one\ntwo").into()));
        assert_eq!(code, 1);
        assert_eq!(
            err.to_string(),
            r#"{"v":1,"type":"error","command":"x","error":{"code":"failed","params":{},"detail":"one\ntwo"}}"#
        );
    }
}
