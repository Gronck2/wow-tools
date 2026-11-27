//! WoW Tools core: everything the program does to a WoW install, with no window, tray, installer or
//! self-update. The `wow-tools` program (CLI, tray) and any other shell sit on top of it. It must
//! never depend on them; see `docs/arch/overview.md`.

#[macro_use]
pub mod i18n;
pub mod app;
pub mod chars;
pub mod config;
pub mod curseforge;
pub mod editmode;
pub mod elvui;
pub mod errors;
pub mod http;
pub mod lock;
pub mod lua;
pub mod macro_fix;
pub mod macro_names;
pub mod macro_store;
pub mod macros;
pub mod maintenance;
pub mod profiles;
pub mod snapshot;
pub mod updater;
pub mod version;
pub mod wago;
pub mod watch;
pub mod wow;
pub mod wowup;
