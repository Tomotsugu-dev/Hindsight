# The island

Overview: [Plugin backend architecture](plugins-architecture.md).

**This doc designs**: how a plugin puts a little content on the island in the middle of the menu bar, such as the time left in a Focus session. The island opens when the mouse moves over it, and a click opens the plugin's page. It covers three things:

- what format the plugin gives the island, and who draws it;
- how much fits in each place;
- which plugin shows when several plugins want the island.

For the methods, see `island.show` and `island.hide` in [③ Requesting data](api.md).

## The places on the island

### Closed

![The closed island](../images/island-closed.svg)

Top: a screen with a notch. Bottom: a screen without a notch.

| Place | What can go there |
|---|---|
| `closed.left` | One [`icon`](island/icon.md), [`timer`](island/timer.md), [`ring`](island/ring.md) or [`text`](island/text.md) |
| `closed.right` | The same |

- At least one side has something.
- Hindsight handles the width of the notch sides and the capsules, and the opening animation. The plugin does not need to know whether the screen has a notch. The width of each side follows its content, and a side with only an icon is round. With a notch, the island reaches out this far on each side of the notch, at most 90. A side with nothing does not reach out. Without a notch, each side is a capsule.
- `text` can only be small, without `sub`: the closed island is only as tall as the menu bar.

### Open

![The open island](../images/island-open.svg)

| Place | What can go there |
|---|---|
| `closed.left` | Stays where it is, the same as when closed |
| `open.topRight` | One small `text`. With a notch, it is right of the notch. Without one, it comes right after `closed.left`. When it is there, `closed.right` is hidden. Otherwise `closed.right` stays |
| `open.left` | Up to 3 components, top to bottom, on the left |
| `open.right` | Up to 2 components in a row, on the right |
| `open.bottomRight` | One small `text`, under the `open.right` row, on the right |

- The width and height of the open island follow its content. It is at most 520 wide.
- `open` can be left out. Then the island does not open when the mouse moves over it, and a click still opens the plugin's page.

## Content format

Example: the Focus plugin starts a 25-minute session. Closed, the left side shows the category icon and the right side shows a countdown. Open, the top strip shows "Working · Code" on the right. The left column shows how long the user has worked today and a row of cells. The right row has the countdown and a pause button, and under the button is the number of distractions.

```json
{
  "closed": {
    "left": { "type": "icon", "name": "Code", "color": "#a78bfa" },
    "right": { "type": "timer", "start": "2026-10-21T10:00:00+09:00", "end": "2026-10-21T10:25:00+09:00", "color": "#fb923c" }
  },
  "open": {
    "topRight": { "type": "text", "text": [{ "text": "Working", "bold": true }, { "text": " · Code", "dim": true }] },
    "left": [
      { "type": "text", "text": [{ "text": "2:40", "bold": true }], "sub": "/ 6:00", "size": "large" },
      { "type": "cells", "colors": ["#a78bfa", "#a78bfa", "#f87171", null, null], "current": 2, "columns": 15 }
    ],
    "right": [
      { "type": "timer", "start": "2026-10-21T10:00:00+09:00", "end": "2026-10-21T10:25:00+09:00", "color": "#fb923c" },
      { "type": "button", "id": "pause", "icon": "Pause" }
    ],
    "bottomRight": { "type": "text", "text": ["Distracted ", { "text": "3", "color": "#f87171", "bold": true }, " times"] }
  }
}
```

Each component has its own doc with its fields, where it can go and its limits:

| Component | What it is | Can go in the closed island |
|---|---|---|
| [`icon`](island/icon.md) | An icon | Yes |
| [`text`](island/text.md) | One line of text | Yes, small text only |
| [`timer`](island/timer.md) | A countdown | Yes |
| [`ring`](island/ring.md) | A progress ring | Yes |
| [`cells`](island/cells.md) | A grid of cells | No |
| [`button`](island/button.md) | A button | No |

Rules of the island:

- Hindsight draws the content, and a plugin cannot put its own HTML there: with HTML, a plugin could draw a fake system dialog.
- Clicking the island does not make Hindsight the front app: otherwise a plugin like Focus would record the click as a switch to Hindsight.
- Unknown components are skipped, with no error: a new plugin on an older Hindsight only shows one piece less.
- The component format is a promise to plugin authors: new components and new fields can be added, and existing fields keep their meaning.
- Colors can only be `#rrggbb`, and Hindsight makes colors that are too dark lighter: a plugin can use the category colors as they are.

## Space in each place

Text is measured by display width: a full-width character counts 2, and any other character counts 1. The limits of each component are in its own doc.

- The limits are set for the widest island: with every place full, the island is still not wider than 520.
- Going over a limit is an error: authors find it while they develop.

## When several plugins want the island

- The plugin that called `island.show` last holds the island. After it takes its content down, the plugin before it comes back.
- When a plugin is turned off or uninstalled, or its background stops, its content is taken down.
- A click on the island opens the page of the plugin that holds it, in its own window.
