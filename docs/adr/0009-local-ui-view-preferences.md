# ADR-0009 · Persist UI view preferences locally, per page

- **Date**: 2026-09-23
- **Status**: Accepted
- **Related**: PR #66 · `src/state/statsView.ts` · `src/state/README.md`

## Context

The stats pages (Today, Week, Month) let the user switch each activity chart
between a bars view ("时段") and a share/pie view ("占比"). The choice was
per-page React `useState` defaulting to bars, so navigating away unmounted the
page and lost it: returning always reset to bars. Nothing outlived the page to
hold the preference.

This is the first view-mode preference we persist, so it sets the pattern for
the next one.

## Decision

We will persist view-mode preferences like the bars/pie toggle **on the device
only, in `localStorage`, keyed per page, and never as part of the synced
dataset.**

Concretely:

- A module-level store in `src/state/` (`statsView.ts`) owns the value and
  reads/writes `localStorage`. Pages subscribe with `useSyncExternalStore` and
  do not hold the value in local state.
- One key per page (`hindsight.stats.view.{today,week,month}`). The three pages
  are independent: changing one does not change the others.
- A missing or invalid stored value falls back to the default (`bars`). Every
  `localStorage` access is guarded, so a blocked store degrades to the default
  rather than throwing.
- The value never enters `sync_outbox` or any cloud file. It is a per-device
  viewing convenience, not shared data.

## Alternatives

- **Local, per-page `localStorage` (chosen)**: survives navigation and restart,
  no sync surface, matches the existing device-filter preference pattern.
- **One preference shared by all three pages**: rejected. The three charts are
  distinct contexts; the user asked for each page to remember its own, and a
  shared value surprises whoever set it on another page.
- **A React Context Provider (`.tsx`)**: rejected. Heavier than needed for a
  leaf global that wraps no subtree; a module-level `.ts` store is lighter and
  already an established style in `src/state/` (see `src/state/README.md`).
- **Sync the preference across devices**: rejected. It is a per-device viewing
  convenience; syncing it adds sync-payload surface and cross-version handling
  for no clear benefit, and works against local-first.
- **Keep it in-component (status quo)**: rejected — this is the bug being fixed.

## Consequences

- **Benefits**: the choice survives page navigation and app restarts; each page
  is independent; no database, migration, sync, or privacy surface.
- **Costs and risks**: the preference is per-device and per-webview — it does
  not follow the user to another device, and clearing site data resets it.
  Acceptable for a viewing convenience. Future per-page view toggles should
  follow this same pattern rather than inventing new storage.

## Data, compatibility, security, and privacy

- **Existing data and migration**: none. New `localStorage` keys; nothing reads
  prior values.
- **Mixed versions and rollback**: no impact. The value is local and never
  synced, so app versions do not affect each other through it. Rolling back
  restores the previous in-session behavior.
- **Irreversible effects**: none.
- **Security and privacy**: no new data leaves the device; no network, no cloud
  file, no sync payload. Local-first preserved.

## Follow-up

- Apply the same store pattern to any future per-page view toggle instead of
  re-adding component-local state.
