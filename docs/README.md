# Project docs

- `tasks/` — what to build and why: one `NNNN-slug.md` per task, numbered together with the plans.
- `plan/` — the approach for a task (`NNNN-slug.md`, same number), written before implementing it.
- `adr/` — architecture decision records. Immutable once accepted: a changed decision gets a new ADR that supersedes
  the old one.
- `arch/` — the current shape of the system (`overview.md`), kept up to date as it changes.
- `images/` — screenshots for the READMEs and the site, rendered by `wow-tools ui-snapshot` (not hand-made).
