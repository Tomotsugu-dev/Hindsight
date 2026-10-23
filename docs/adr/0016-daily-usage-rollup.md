# ADR-0016 · Speed up the usage reports with a table of daily totals

- **Date**: 2026-10-24
- **Status**: Proposed
- **Related**: PR #170

## Goal

Make the backend queries faster for four pages: Daily, Weekly, Monthly and All time. All time takes 445 ms from click to chart, and 370 ms of that is the backend queries. The target is 150 ms or less.

## The two queries to speed up

| What the page shows | Backend function | All time now |
|---|---|---|
| A bar for each day. The frontend adds up the bars for the time per category and the total time | `day_category_time` | 234 ms |
| Top apps: each app's total time | `top_apps` | 136 ms |

Not in scope: the app detail page (`app_detail`) and the hourly bars on Daily.

## Bottleneck

Both functions take the same three steps every time:

1. Read every record in the date range. All time has 240,000 rows.
2. Find the category of each row.
3. Add up the seconds per day and category, or per app.

## Design

### 1. New table `daily_usage`: daily pre-aggregation

Each row holds the total seconds for one date, device, process and website. A record that crosses midnight is split at midnight first, and each part counts toward its own day. For example, `activities` has these three rows:

| local_date | device | process | url_host | seconds |
|---|---|---|---|---|
| 2026-10-21 | mac-a | Google Chrome | github.com | 1200 |
| 2026-10-21 | mac-a | Google Chrome | github.com | 2200 |
| 2026-10-21 | mac-a | Code | (none) | 12600 |

In `daily_usage` they become two rows:

| local_date | device_id | process_name | url_host | secs |
|---|---|---|---|---|
| 2026-10-21 | mac-a | Google Chrome | github.com | 3400 |
| 2026-10-21 | mac-a | Code | `''` | 12600 |

All time goes from 240,000 rows to 4,863. The primary key is the first four columns, and the table is `WITHOUT ROWID`. A record without a website stores `''`, not NULL. NULLs are never equal in a primary key, so their seconds could not be added to the same row.

### 2. The queries read only this table, and find categories the same way

`day_category_time` and `top_apps` read only `daily_usage`. Step 2 uses the same SQL as before (`from_stats_category_sql`). Only its source changes from `activities` to `daily_usage`. The separate query for records that cross midnight is removed.

### 3. Categories are not stored; the queries find them

When the user changes a website rule or an app's category, the next page load sorts these 4,863 rows by the new rules. The table does not change. If it stored categories, every rule change would mean rebuilding the whole table.

### 4. Keeping the table current: triggers note the changed days, and the queries recompute them

| When | What happens |
|---|---|
| A row in `activities` is inserted or deleted, or its date, device, process, website, seconds, start or end time, or ignore flag changes | A trigger writes the dates the row covers (from its start date to its end date, both before and after the change) into `daily_usage_stale` |
| Before a query | Recompute the noted days into `daily_usage`, only those days, then clear the notes |

- **Why triggers**: More than ten places write to `activities`: capture, sync pull, ignore rules, deletion, startup repair and others. The database makes sure every write is noted, including writes added later.

### 5. Not synced

Each computer builds this table from its own `activities`, which already holds the rows synced from other devices.

## Results (measured on a copy of a real database)

| | Before | After |
|---|---|---|
| All time, report SQL | 391 ms | 8 ms or less |
| Monthly, report SQL | 55 ms | 1.8 ms or less |
| Recompute today before a query | — | 0.8 ms (2.7 ms on the day with the most records) |
| First query after the ignore rules change: recompute all 160 days | — | 134 ms |

The two recompute times were measured on an in-memory database. They will be slower on a real disk.

- **Migration v46**: Creates the `daily_usage` and `daily_usage_stale` tables and the triggers, then notes every date as changed. The first query after the upgrade builds the whole table.
- **Disk**: The table is under 1 MB and grows by about 1 MB a year.

## Compatibility and rules for later changes

- **Going back to an older version**: Older versions do not read the two tables, but the triggers stay in the database and keep noting dates while an older version runs. After the next upgrade, the first query recomputes those dates, so the table is still correct.
- **Do not write to `activities` with `INSERT OR REPLACE`**: When it deletes the old row, the delete trigger does not run by default. If the old row and the new row have different dates, the old row's day is not noted and stays wrong.
- **A migration that rebuilds the `activities` table must recreate the triggers.**
- **If the way of recomputing changes, a migration must note every date as changed** so the whole table is recomputed. A day that was computed wrong does not fix itself if it never changes again.
- **Privacy**: Nothing new is uploaded. All the data in the table is already in `activities`.
