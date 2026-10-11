# Proposal for an activity continuity view

Status: proposed for discussion. The placement has been explored in a local UI prototype, but the wording and counting rules are not ready for acceptance. This document proposes a direction rather than a finished attention measurement feature.

## Why add this view

Time distribution and category share answer how much time was spent in each activity. They do not show whether that time consisted of a few sustained runs or many short returns. Two days can have the same application totals and very different patterns of switching.

The proposed view would help a person inspect those patterns using the activity history Hindsight already records. It should work equally for writing a paper, playing a game, watching a video, or another chosen activity. It should not assign a higher value to work than leisure or turn application categories into a productivity grade.

The product question is whether a small, understandable continuity view is useful alongside the existing reports. It is not whether Hindsight can infer someone's mental attention from their foreground application.

## What the observations mean

Application changes, task changes, and loss of attention are different things. Writing a paper can require an editor, PDF reader, reference manager, and browser. A game may involve a guide and voice chat. Conversely, a person may leave one window open while looking away or using another device.

Task-switching experiments support costs in particular cognitive tasks, but do not establish a universal penalty for each window change. Research on task-centric application switching also shows why several applications can support a single goal. These findings motivate careful definitions and contextual interpretation, rather than a universal focus score. See [Rubinstein et al. 2001](https://doi.org/10.1037/0096-1523.27.4.763) and [Jahanlou et al. 2023](https://www.research.autodesk.com/app/uploads/2024/02/Task-Centric-Application-Switching_GI23.pdf).

There would be no 0–100 psychological attention score, good/bad ranking, or default productivity weighting in this proposal. Automatic task inference and interruption reminders should be separate decisions.

## Proposed UI placement

Reuse the Daily, Weekly, and Monthly report card. Add a third view beside the existing Time and Share controls. Keep the same date navigation, device selector, themes, and responsive layout.

```text
Daily / Weekly / Monthly

+---------------------------------------------------------------------+
| Activity continuity                                      [title]     |
| [Time]  [Share]  [Continuity*]     [Device]   [Previous] [Period] [Next]|
|---------------------------------------------------------------------|
| Whole period · app-level*                       [Quick-return limit*]|
|                                                                     |
| App changes/h* | Longest app run* | Median app run* | Quick returns*  |
|       --       |        --       |       --        |       --        |
|                                                                     |
| Recorded coverage and a neutral comparison with the previous period  |
| [How this is counted]                                               |
+---------------------------------------------------------------------+
| Existing application and category rankings                           |
+---------------------------------------------------------------------+

* Provisional labels and controls; no final wording is proposed here.
  Values are placeholders, not a screenshot or export of user history.
```

The current prototype calls the tab "Focus", but "Continuity" or "Activity continuity" may describe the observable data more accurately. Please consider this name open for discussion.

Only one view is displayed inside the card at a time. The third view should not create a second report card below the chart. It should not silently inherit an hour selection, category drill-down, or application hover from another view. Initially its scope would be the selected period and device; application-specific or task-specific scopes need their own explicit controls.

The current prototype has too much explanatory text. A better direction may be concise metric labels, a short coverage note, and an expandable explanation of the counting method. On narrow windows the controls can wrap and the four values can form two columns. Empty or unavailable data should be shown as such, not as perfect focus or a fabricated zero.

## Candidate metrics and unresolved definitions

These are candidates to discuss, not agreed product rules.

| Candidate | Possible observable definition | Main decision still needed |
|---|---|---|
| Application change rate | Changes in the chosen application identity divided by recorded device-hours | Process, app group, window, website, or user-defined task identity? Which changes are useful to count? |
| Longest continuous run | Longest contiguous period with the same chosen identity | What ends a run? How should necessary tool changes, idle time, sampling noise, and gaps be represented? |
| Typical continuous run | Median run duration | Is the unweighted median understandable, or does it overemphasize short necessary actions? Should a distribution or time-weighted statistic be shown instead? |
| Quick returns | A → B → A where the middle run is short | Count overlapping patterns or distinct excursions? What label makes clear that a return can be useful, not a distraction? |

The prototype exposes 10, 30, and 60 seconds for the middle run. These are experimental engineering parameters, not validated thresholds for distraction. The quick-return count does not remove the underlying changes or turn them into a penalty. We should decide whether this control is useful at all before retaining it.

Period comparisons should remain descriptive. A different task mix, partial current period, tracking coverage, or device usage can change a rate without demonstrating better or worse attention.

## Data and implementation direction

A first implementation could be a read-only report over existing activities, with the calculations in a separate report module and a thin IPC command. A shared frontend component could render the result in the three existing pages. This should avoid changing capture, migrations, synchronization, or stored user settings solely for the initial view.

Important constraints include:

- Activity rows are periodically split even when the application stays the same. Row count is not a change count.
- Current capture samples the foreground approximately every five seconds. Changes between observations can be missed, so a metric cannot promise complete window-event counts.
- A logical application may include several processes. The identity rule must be consistent with existing app grouping, but app groups still do not identify tasks.
- Hidden or excluded history, unknown intervals, and unsealed rows must not be bridged into invented continuity. Zero-duration records need a clear policy; not every zero necessarily means the same thing.
- Each device needs an independent sequence. Interleaving two devices must not manufacture switches. Device-hours may overlap and should not be presented as a person's wall-clock hours.
- Recorded timestamps can use different offsets. Date clipping and ordering need actual times rather than lexicographic comparisons of timestamp strings.
- Summing each row's integer `duration_secs` can differ from summing precise interval lengths. A common duration convention should be chosen before displaying totals that appear directly comparable.

No new screenshot collection, OCR, cloud upload, or model call is needed for this direction. Actual foreground-window events, task sessions, or semantic task classification would be later scope changes, not implied by this document.

## How to evaluate the direction

First validate observable behavior with synthetic sequences: periodic splits, app grouping, short returns, gaps, ignored intervals, overlapping devices, midnight boundaries, and incomplete records. Then check whether the labels and explanations let users correctly interpret those observations.

If the product later makes claims about experienced attention or task performance, it needs task context and validation against suitable user feedback or outcomes. Agreement between an implementation and its own formula would not establish that validity.

## Discussion requested

1. Is continuity information useful enough to add to the existing reports?
2. Does the third-view placement fit the existing UI, or should it live elsewhere?
3. Which identity, run boundaries, quick-return semantics, and duration convention should be used?
4. Which labels would explain these observations without implying a mental-attention or productivity assessment?

The local reference implementation will be linked in the PR conversation for discussion. It is not part of this PR's file changes and should not be treated as a ready-to-merge feature.
