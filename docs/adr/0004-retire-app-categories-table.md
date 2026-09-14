# ADR-0004 · Drop `app_categories`, stop publishing it, and remove its cloud file

- **Date**: 2026-09-14
- **Status**: **Accepted**
- **Related**: #32 (stop local writes) · #36 (stop consuming it) · ADR-0001 (deletion propagation)

## Context

`app_categories` stores the mapping from process names to categories. It is a
legacy structure from before app groups existed and has been retired one layer
at a time:

| Stage | Current state |
|---|---|
| Local reads | None since v0.7.0 |
| Local writes | None since v0.8.23 |
| Pull and merge | None since #36 |
| Push and publish | **Still active**: derive `device.<id>.app_categories.json` from app groups for devices older than v0.7.0 |
| App deletion and data clearing | **Neither touches this table** |

The final row is the problem. Nothing reads, writes, or deletes this table. When
a user deletes an app or removes it from a group, its process name remains in
the table indefinitely. This device's cloud file retains the same historical
process names. It is one of the deletion remnants addressed by ADR-0001, and it
can be removed without designing a new protocol.

The only reason to keep the table is to deliver categories to devices older than
v0.7.0, released on 2026-05-17.

**If we make no decision, process names remain in the local table and cloud file
after app deletion, with no code path capable of removing them.**

## Decision

Make three changes together:

1. **Stop publishing** `app_categories.json`.
2. **Drop the table immediately** with a new
   `DROP TABLE IF EXISTS app_categories` migration.
3. **Delete this device's cloud file**,
   `device.<this-device-id>.app_categories.json`. Check once during the first
   push after each startup. After deleting the file or confirming that it does
   not exist, do not check again during that run.

## Alternatives

| Question | Option | Why chosen or rejected |
|---|---|---|
| Continue publishing? | **Stop (chosen)** | The only beneficiaries are versions already four months old, while the cost is an entire derivation path and the remnants it creates |
| | Continue | Preserves category sync for old devices, but requires keeping both the table and its remnants |
| How should the table be handled? | **Drop it now (chosen)** | Removes historical process names immediately |
| | Clear it now and drop it next release | Preserves rollback to v0.8.22 and earlier, but requires maintaining an empty table for another release |
| What about the cloud file? | **Delete this device's file (chosen)** | Otherwise the local copy is cleaned while historical process names remain in the cloud |
| | Leave it alone | Simpler, but leaves the cleanup incomplete |

Two mechanisms were considered for cloud deletion: have the migration write an
outbox row for the push path to execute, or put a one-time switch in the sync
engine and execute it during the first push after startup. We chose the latter.
The former would add a non-upload special case to the upload loop; the latter is
clearly temporary code that can later be removed as one block.

## Consequences

**Devices older than v0.7.0 no longer receive new categories.** Their category
page relies on this file to populate the app list. The release notes must state
this limitation.

**Rolling back to v0.8.22 or earlier makes app categorization fail.** The
migration runner does not detect that the database is newer than the program, so
those versions still start, but their categorization path writes to the removed
table and fails. Rolling back to the then-current release, v0.8.23, still works:
only its sync code touches the table, and it logs a warning and skips the failed
row.

**Cloud cleanup reaches only each upgraded device's own file.** Every device
deletes only `device.<this-device-id>.app_categories.json`. Files belonging to
devices that have not upgraded remain until those devices upgrade.

**Each startup adds one list-files request** until the one-time cleanup code is
removed.

## Impact on user data

- **Existing data**: The local table and its historical process names are
  deleted. No code reads the table, so functionality is unaffected. Old
  `app_category` rows left in the outbox are treated as unknown entities, logged
  with a warning, and removed.
- **Migration**: A new migration drops the table in the same transaction that
  records the schema version. A fresh installation creates the table while
  replaying historical migrations, then drops it.
- **Rollback**: v0.8.23 remains usable, with missing-table warnings in the sync
  log. In v0.8.22 and earlier, categorization fails.
- **Irreversible effects**: Dropping the table and deleting the cloud file cannot
  be undone. They remove only historical copies that nothing reads; the source
  of truth for categories remains `app_groups.category_id`.

## Follow-up

- **Release notes**: Warn that devices older than v0.7.0 no longer receive
  category updates.
- **Cloud cleanup code**: Remove it after active devices have probably upgraded.
  Its `TODO` must point back to this ADR.
- **ADR-0001**: One source of deletion remnants has been eliminated.
