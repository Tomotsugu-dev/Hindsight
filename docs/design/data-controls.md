# Data controls

What a user can do to make an app contribute less, and what each control actually stops. Four mechanisms exist today, one is missing, and they are easy to mistake for each other because three of them look the same in the UI: the app goes away.

An activity record passes four stages, and a control can stop it at any of them:

```text
    capture ────────→ counted ────────→ uploaded ────────→ kept
       │                 │                  │                │
    screenshot        ignore rule        (nothing)      delete the app
    policy            hidden category
```

## The four, side by side

| Control | Stops | Granularity | Syncs to other devices | Reversible |
|---|---|---|---|---|
| Screenshot policy | the screenshot | URL, or app name / window title keyword | No — a local setting | Yes, drop the keyword |
| Ignore rule | counting towards stats | process name + optional window-title keyword | No — a local setting | Yes, drop the rule and recompute |
| Hidden category | counting towards stats | one whole app | **Yes** — it is a category assignment | Yes, move the app to another category |
| Delete the app | everything stored on this machine | one whole app | **No** — peers and the cloud keep their copies | No |
| *(missing)* | *uploading* | — | — | — |

## Screenshot policy

`capture/screenshot_policy.rs`, driven by the keyword lists in Settings → Privacy.
A match means no image is written to disk. The activity row is still stored, the
time is still counted, and the row is still uploaded — the only thing that never
exists is the picture.

Two lists: one matched against the browser URL (ten defaults covering login,
auth and password paths), one matched against the app name or the window title
(empty by default).

## Ignore rule

`capture/ignore.rs`, driven by the rules in Settings. A match sets
`activities.excluded = 1`, and reports, exports, AI summaries and chat queries
skip the row — eleven queries filter on it. The row is still stored, still
screenshotted, and **still uploaded**.

The flag is not part of the sync payload, and incoming rows from a peer are
tagged with *this* device's rules as they are inserted. The position is
deliberate: the data is shared, the interpretation is local. Each device decides
for itself what counts.

Because it is a local view, it is fully reversible: delete the rule, run
`repo::activities::reapply_ignore_rules`, and the history counts again.

## Hidden category

Assigning an app to the built-in `hidden` category. Eleven report and summary
queries carry `g.category_id IS NOT 'hidden'`, so the app disappears from every
chart and every AI summary. Nothing is deleted.

This is the one exclusion that **syncs**, because the category lives on the app
group, and app groups sync. Hiding an app on one device hides it everywhere. That
is the difference from an ignore rule, and it is the reason both exist: "I don't
want to see this anywhere" is a different statement from "this machine shouldn't
count it".

## Delete the app

`repo::app_groups::purge_with_data`, the "Delete anyway" button on the pairing
page. Physically removes, on this machine only: activity rows, icons, executable
paths, the OCR text in the memory DB, and the screenshot files only this app
referenced. The group itself is soft-deleted so the deletion reaches peers as a
tombstone.

What it does not reach:

- the day files already in the cloud (`activities.<date>.ndjson`, and
  `memory.<date>.ndjson` when screen-memory sync is on) — those are only rewritten
  when that day changes again, which for past days never happens;
- other devices, which keep their own copies alive and will push them back;
- generated daily and weekly AI summaries, which are stored text that mentions
  the app by name;
- chat history, if the conversation mentioned it;
- OCR text captured while a *different* app was in the foreground — screenshots
  cover the whole screen, so that text belongs to the other app's session and no
  amount of deleting this app will find it.

## The missing one: never upload

There is no way to keep a given app's records out of the cloud. The only
all-or-nothing lever is turning sync off entirely.

Overloading the ignore rule with it would be wrong on two counts. It would let
one device's local setting decide what another device is ever allowed to see,
and it would give a reversible setting an irreversible side effect: drop the rule
later and the local stats come back, but the months of day files that were never
uploaded stay missing, because past days are never rewritten.

So it belongs on its own axis, alongside the other three, and it pairs with a
deletion that propagates: one keeps future records out, the other clears what is
already there.
