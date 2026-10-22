# ring: a progress ring

![ring](../../images/island/ring.svg)

Left: closed. Right: open. In both, `value` is 0.75.

```json
{ "type": "ring", "value": 0.75, "text": "75%", "color": "#34d399" }
```

| Field | Required | Notes |
|---|---|---|
| `value` | Yes | The progress, from 0 to 1 |
| `text` | No | Right of the ring when closed, and in its middle when open |
| `color` | No | The color of the ring, `#rrggbb`. White if left out |

- At 1 the ring is a full circle: 1 means done. The ring of a [`timer`](timer.md) leaves a gap between its ends, and `ring` does not.

## Allowed places

| Place | Looks like |
|---|---|
| `closed.left`, `closed.right` | A 14 px ring |
| `open.left`, `open.right` | A 52 px ring |

## Length and value limits

- `text` is at most 6 wide. For how width is measured, see "Space in each place" in [The island](../island.md).
- `text` cannot be empty, and cannot have line breaks or other control characters.
- A `value` outside 0 to 1 is an error.
