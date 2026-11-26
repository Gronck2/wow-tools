# 0002. A core crate and shells; a CLI with JSON for non-Rust shells

Status: accepted (2026-11-27)

## Context

WoW Tools was one binary: the operations on a WoW install (snapshots, restore, profiles, cleanup, macro check, addon
updates), the CLI and the Direct2D tray UI. A second front end may come next to it (Overwolf or ow-electron: its own UI,
store and updates). Two front ends on separate copies of the logic would drift, and two processes changing one `WTF`
folder at once would corrupt it.

## Decision

- Split the code into a library crate `wow-tools-core` (`crates/core`) and **shells**. The shell today is the package
  `wow-tools` (exe, CLI with clap, tray, windows, self-update, installer integration, `schedule`, `diag`). The core has
  no window, tray, installer or self-update and never imports them; it carries the interface strings (`locales/`) and
  the data tables (`data/`). The workspace root stays the `wow-tools` package so `target\release\wow-tools.exe`,
  `build.rs`, the installer and the workflows keep their paths.
- All writers of a WoW folder go through one **file lock per flavor folder** (`lock::acquire`), fail-fast, so the tray,
  `watch`, a scheduled run, the CLI and any other shell exclude each other. Details in `docs/arch/overview.md`.
- **How a shell reaches the core.** A Rust shell links the library. A non-Rust shell (Electron, Overwolf) runs the
  `wow-tools` CLI as a child process and reads **JSON output** (to be added to the CLI in its own task; nothing of it
  exists yet). Recommended over a native binding (napi) because the CLI is already the stable contract, the process
  boundary contains crashes and keeps the lock semantics, and no native addon has to be built per Electron version.
  Cost: a process start per call and a JSON schema to keep stable.
- The core stays Windows-only for now (registry, WinHTTP, WinEvent hook); the code keeps its non-Windows stubs.
- `clap` is an optional feature of the core (`Kind` and `chars::Part` as command-line values); only the program
  turns it on.

## Consequences

- Core tests run alone (`cargo test -p wow-tools-core`); every gate and workflow uses `--workspace`.
- A change of the core's behaviour is visible to every shell; the CLI output (and later its JSON) is the public contract.
- Not decided: publishing the core to crates.io (the manifest says `publish = false`), a virtual workspace root (revisit
  with a second program crate), the JSON schema, an Electron/Overwolf shell itself.
