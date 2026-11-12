# Changelog

What changed in each version of WoW Tools. The release notes on GitHub are taken from here.

## [Unreleased]

- Profiles no longer carry key bindings, click bindings and macros: the target character keeps its own. Applying
  a profile used to replace the target's macros with the source's. Profiles saved before still have them; they are
  not applied.
- Edit Mode layouts of a profile are added to the target's own instead of replacing them, and its specializations
  are switched to them. A layout already on the target's account is used as it is; a name that is taken gets the
  source character's name in brackets. Profiles now also keep the account's layouts, so they carry over to
  characters on another account too. A layout that does not fit (5 per character) is named in the result.

## [0.5.1] - 2026-11-08

- WoW Tools no longer crashes when it closes (Quit, a self-update restart, an upgrade) while the panel, the menu or
  the settings window is open, e.g. a panel pinned with the paperclip. Windows recorded it as an application error
  and kept a crash dump each time.

## [0.5.0] - 2026-11-08

- Profiles: characters are shown with a faction badge before the name and the name in its class color, in the
  character lists and the saved profiles. Both are read from addons that keep them (WorldQuestTracker,
  AllTheThings, AltVault, ElvUI); a character no such addon has seen stays plain.
- A lock next to the paperclip in the panel header: open, the panel can be dragged by its header; it then opens
  where it was left (at the tray when that monitor is gone), and closing the lock keeps it there. Closed when WoW
  Tools starts.
- The header buttons (new version, lock, paperclip, folder, settings) say what they do when you hover them. When
  there is no room, the new version button shows only its icon.
- The panel is no longer always on top: other windows can cover it. The paperclip only decides whether it closes
  when you click elsewhere. A click on the tray icon brings a covered pinned panel to the front (and closes it when
  it is in plain sight, as before).

## [0.4.1] - 2026-11-03

- Settings → General → Diagnostics: a Copy button puts the version, settings, addon managers, addon update history
  and the end of the log on the clipboard (the user folder replaced with `%USERPROFILE%`), to send when something
  does not work.
- An addon update that could not run because CurseForge or WowUp was already open with its own auto-update off no
  longer says "No updates": the history shows it as *not updated* (in yellow), and a scheduled update tries again every
  half hour instead of waiting for the next one. A repeated failure of a scheduled update pops up once, not every time.

## [0.4.0] - 2026-11-03

- Sharp text on high-DPI screens: at 150-200% display scaling the windows were drawn at 100% and stretched by
  Windows, so the text was blurry.
- Selected options in Settings (versions, days, folders by date, ...) have a check mark, and a grey Windows accent
  color is replaced with blue: with a grey accent a selected option looked disabled.
- Backups every N days (2 to 30, counted from the last snapshot) at a set time, next to "after the game" and
  "by weekday".
- Addon updates can run once when WoW Tools starts (with Windows, before you play).
- Settings → Addons: the "update automatically" switch is at the top, under a note that what gets updated (and to
  which version: release, beta, alpha) is decided by the addon manager's own settings, and a warning that managers
  overwrite addon files edited by hand and how to keep such addons out of updates.
- Settings → Screenshots says that screenshots are moved only with the Move button of the panel's Cleanup tab, and
  every game version can have a folder of its own, anywhere.
- While a snapshot is written, the panel and the tray tooltip show what is saved and how far it is
  ("Snapshot Retail · AddOns: 45%").
- A note about a change on one Settings tab no longer stays at the bottom of the others.
- The Profiles tab warns that applying a profile is at your own risk (a WTF snapshot is saved before it).

## [0.3.2] - 2026-11-01

- WowUp updates no longer show the Wago ad consent window: WowUp 2.2x opens it on every start, even hidden,
  above everything. WoW Tools now hides the windows of the WowUp it started while the update runs.
- The pinned panel no longer leaves its shadow on screen when another window (or a fullscreen game) covers it.
- Warcraft III Reforged and other Blizzard games installed next to WoW are no longer taken for a WoW version
  (they have a `_retail_` folder too).
- Submitted to winget as `Gronck2.WoWTools` (in review).
- Website in five languages: https://wow-tool.gronck.com.

## [0.3.1] - 2026-10-31

- Three downloads per release: the program itself (`WoW-Tools-x.y.z.exe`, what self-update installs), an installer
  (`WoW-Tools-Setup-x.y.z.exe`, per user, no admin rights, with "Start with Windows" and an uninstaller) and a
  portable zip that keeps its settings and data next to the exe.
- Every download has a SHA-256 line in the release notes and a build provenance attestation
  (`gh attestation verify`); README "Verifying downloads" explains how to check them.
- The installer closes a running WoW Tools first (and waits while it is busy) and starts it again after a silent
  upgrade.
- The exe carries version information (Properties → Details).
- License: MIT OR Apache-2.0.

## [0.2.3] - 2026-10-15

- Settings → General: a switch for tray notifications.
- A paperclip in the panel header pins the panel: it stays open when it loses focus and after the folder and
  settings buttons.

## [0.2.2] - 2026-10-14

- ElvUI is updated from tukui.org during the addon update (no addon manager carries it), also with no manager
  installed. Its folders are replaced whole, which clears files an older version left behind; a mixed-version
  install is reinstalled. A switch in Settings → Addons turns it off.
- ElvUI has its own row in the update history, with the old and new version.

## [0.2.1] - 2026-10-14

- A profile apply can be undone from the panel (the "before profile" restore point).
- Confirmations and pick lists stay on the same restore point, profile or character when the lists change, instead
  of acting on whatever moved into their row.
- A recent safety snapshot covers the next action only while WTF is unchanged since then.
- SavedVariables that are not UTF-8 no longer stop saving or applying a profile.
- Characters played after the game last wrote its character list are no longer hidden.

## [0.2.0] - 2026-10-14

- Profiles: save a character's interface (its settings, its SavedVariables and the profile it uses in AceDB addons)
  and put it on other characters, on the same or another account. `wow-tools profile`.
- Character lists show only existing characters, last played first, with a search field.
- Cleanup tab in the panel: settings of removed addons, `.bak`/`.old` copies, old combat logs, screenshots (moved to
  Pictures, by date and game version), cache. Settings get a Screenshots tab. `wow-tools clean`.
- Game patches: when a game version is patched and not started yet, WTF and AddOns are saved first, and a
  notification lists addons not made for the new version (also in Settings → Addons and `wow-tools outdated`).
- Self-update from GitHub releases: a button in the panel header, the menu and Settings → General;
  `wow-tools self-update`. The download is checked against its SHA-256.
- The panel shows the last five addon updates; an update has one time (its start).
- The settings window fits the open tab.
- New tray and app icons: grey with no game, gold while a game runs, a dot while working or after a failure.

## [0.1.0] - 2026-10-13

- First release: a tray app and CLI that backs up WTF and Interface\AddOns for every installed WoW version (after
  the game closes, or on a schedule), restores them, copies settings between characters and updates addons through
  CurseForge, WowUp or the Wago App.
- Interface and documentation in English, Russian, German, French and Spanish.
