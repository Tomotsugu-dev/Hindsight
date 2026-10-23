# cells: a grid of cells

![cells](../../images/island/cells.svg)

Only in the open island. In the image, each row has 10 cells, there are 14 cells, and the 8th cell is the current one.

```json
{
  "type": "cells",
  "colors": ["#a78bfa", "#a78bfa", "#a78bfa", "#a78bfa", "#a78bfa", "#f87171", "#a78bfa", "#a78bfa", null, null, null, null, null, null],
  "columns": 10,
  "current": 7
}
```

| Field | Required | Notes |
|---|---|---|
| `colors` | Yes | One color for each cell, `#rrggbb`. An empty cell is `null` |
| `columns` | Yes | The number of cells in a row |
| `current` | No | The index of the current cell, counting from 0. This cell has a white outline |

- A cell is at most 10 px and does not grow with the panel: three rows must fit in the panel.
- When `open.right` does not fit, the left column gives up space and the cells get smaller.

## Allowed places

- `open.left`, `open.right`. Cells do not fit in the closed island.

## Count limits

These are errors:

- `columns` is not 1 to 20.
- The cells take more than 3 rows. So `colors` can have at most 60 items.
- `colors` is empty.
- `current` is not one of the cells.
