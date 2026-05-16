# Lattice — AGENTS

This file is the mandatory operating contract for any agent working in this repository.

## Mandatory Startup Routine

Before taking any action in this repository, the agent must:

1. Read `JOURNAL.md`.
2. Read `README.md`.
3. Read `docs/roadmap.md` when scope or milestone boundaries matter.
4. Follow this file before any other repo-local workflow preference.

No work may begin until `JOURNAL.md` has been read for current context.

## Mandatory Journal Discipline

Every meaningful change must be written to `JOURNAL.md`.

Required behavior:

- Add a journal entry after each completed work session.
- Record what changed, why it changed, and any follow-up work or known gaps.
- Record checks run, checks skipped, and the exact reason for any skipped command.
- Keep entries chronological and concise.
- Treat the journal as the authoritative project changelog during active development.

Leaving changes undocumented in `JOURNAL.md` is a project rule violation.

## Mandatory README Discipline

`README.md` must be updated after each major step.

Requirements:

- Keep `README.md` functional, direct, and easy to understand.
- Remove chaff, stale milestone language, and vague marketing filler.
- Keep setup, current status, run instructions, and current feature boundaries accurate.
- Update workflow notes or feature status when the user-facing behavior materially changes.
- Update screenshots when practical after material UI changes, but missing screenshots do not block completion unless the milestone explicitly requires them.

If a change materially affects how the project is built, run, understood, or evaluated, `README.md` must be reviewed and updated as needed in the same session.

## Standards Are Mandatory

All agents must strictly follow project standards. These are not suggestions.

Primary standards source:

- `docs/agent_rules.md`

Non-negotiable enforcement:

- Respect milestone boundaries in `docs/roadmap.md`.
- Preserve the mouse-first product direction.
- Keep Lattice mouse-first, visually intentional, polished, and cyberpunk-ish rather than falling back to plain GTK defaults where the milestone expects visual quality.
- Keep the default UX icon-grid-first unless a milestone explicitly changes that.
- Keyboard shortcuts are optional accelerators, not the primary interaction model.
- Use GTK4 + GIO/GLib native APIs for real file behavior.
- Keep slow file operations off the GTK main thread.
- Route styling through stable CSS classes in `themes/default.css`.
- Do not scatter hardcoded visual styling through Rust code unless GTK specifically requires it.
- Preserve existing CSS class names unless they are being intentionally migrated and the theme/docs are updated in the same pass.
- All popup dialogs must use the shared dialog layout helpers and shared dialog CSS classes. Keep popup width stabilized through the shared helper's GTK width request path; do not hand-roll dialog sizing in ways that let long prompt text or initial entry text change the popup width after it appears.
- Do not introduce keyboard-first flows.
- Never silently overwrite.
- Never make permanent delete the default destructive action.
- Use Move to Trash as the normal delete behavior where available.
- Require explicit confirmation for permanent delete.
- Make file-operation failures clear and safe.

Any change that violates the project standards must be corrected before the session ends.

## Build and Verification Rules

- After any Rust code change, run `cargo fmt` and `cargo check`.
- After UI, startup, GTK, theme-loading, config-loading, or runtime-behavior changes, also run `cargo run` when practical.
- If a command cannot be run, the agent must say exactly why in `JOURNAL.md` and in the final report.
- Do not claim a feature works unless it has been implemented and at least manually checked.
- If behavior is placeholder-only, label it clearly in code, docs, and `JOURNAL.md`.
- Do not mark roadmap items complete unless their acceptance criteria are actually met.

## Implementation Expectations

- Prefer narrow, milestone-appropriate changes.
- Do not add future-phase features opportunistically.
- Do not silently remove, disable, or downgrade working behavior to make a milestone easier unless explicitly instructed.
- If something must be removed temporarily, document why it was removed and what is needed to restore it.
- Keep docs aligned with the actual codebase state.
- When behavior changes, update tests/checks/docs in the same pass where practical.
- If the repo state and the docs disagree, fix the docs or the code immediately.

## Audit and Fix Discipline

- When given an audit prompt, inspect first, list issues, then fix.
- Do not use audit sessions to add future-phase features.
- Fix scope violations directly instead of building around them.

## Final Response Requirements

Every agent session must end by reporting:

1. What changed.
2. Checks run.
3. Known gaps.
4. Files modified.
5. Whether `cargo fmt`, `cargo check`, and `cargo run` succeeded, or exactly why they were skipped.

## End-of-Session Checklist

Before ending a session, the agent must ensure:

1. `JOURNAL.md` has a new entry for the work performed.
2. `README.md` has been reviewed and updated if the step was major.
3. Relevant standards in `docs/agent_rules.md` were followed.
4. Required verification commands were run, or skipped reasons were documented exactly.
