# timer: a countdown

![timer](../../images/island/timer.svg)

Left: closed. Right: open. In both, a quarter of the time is gone. The ring gets shorter as time passes, and the time left is next to it: `h:mm:ss` from one hour on, and `m:ss` below one hour.

```json
{ "type": "timer", "start": "2026-10-21T10:00:00+09:00", "end": "2026-10-21T10:25:00+09:00", "color": "#fb923c" }
```

| Field | Required | Notes |
|---|---|---|
| `start` | Yes | The start time, in RFC 3339, the same as `at` in `focus.changed` |
| `end` | Yes | The end time, in the same format |
| `pausedAt` | No | The time it paused, in the same format. With it, the timer stays at the time left at that moment |
| `color` | No | The color of the ring, `#rrggbb`. White if left out |

- Hindsight counts the timer down, so the plugin does not need to call every second. To pause, the plugin calls `island.show` again, with `pausedAt` on the timer.
- At 0, the timer stays at 0 until the plugin changes the content.
- The two ends of the ring leave a gap: even with all the time left, it still looks like a ring.

## Allowed places

| Place | Looks like |
|---|---|
| `closed.left`, `closed.right` | A 14 px ring and small text |
| `open.left`, `open.right` | A 20 px ring and large text |

## Value limits

These are errors:

- `start` is not before `end`.
- `pausedAt` is not between `start` and `end`.
