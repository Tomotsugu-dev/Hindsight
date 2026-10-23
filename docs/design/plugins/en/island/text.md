# text: one line of text

![text](../../images/island/text-en.svg)

Left: small text in the closed island. Right: the open island. The top line is large text with `sub`, and the bottom line is small text in runs.

```json
{ "type": "text", "text": "Focus" }
```

```json
{ "type": "text", "text": [{ "text": "2:40", "bold": true }], "sub": "/ 6:00", "size": "large" }
```

```json
{ "type": "text", "text": [{ "text": "Working", "bold": true }, { "text": " · Code", "dim": true }] }
```

| Field | Required | Notes |
|---|---|---|
| `text` | Yes | A string, or a list of runs |
| `sub` | No | Smaller grey text after it. A string only |
| `size` | No | `"small"` (the default) or `"large"` |

Each run is a string, or:

| Field | Required | Notes |
|---|---|---|
| `text` | Yes | The text of this run |
| `color` | No | `#rrggbb` |
| `dim` | No | `true` makes it grey |
| `bold` | No | `true` makes it bold |

- Styles are given per run, not as markup in the string. Markup needs escape rules: Telegram's MarkdownV2, for example, has more than ten characters to escape. Runs need no escaping. Microsoft's Adaptive Cards also use a list of runs (TextRun), and a plain string is a short form without styles.
- The only styles are color, grey and bold. The font and the font size cannot change: the limits are set for Hindsight's font and its two sizes, and the same text is wider in another font. A plugin also cannot ship font files, and a font the user does not have is quietly replaced.

## Allowed places

| Place | Limit |
|---|---|
| `closed.left`, `closed.right` | Small text only, without `sub`: the closed island is only as tall as the menu bar |
| `open.topRight`, `open.bottomRight` | Small text only |
| `open.left`, `open.right` | Large or small text |

## Length limits

For how width is measured, see "Space in each place" in [The island](../island.md).

| Where | Limit |
|---|---|
| Closed | 6 |
| Large text | 8 |
| `sub` after large text | 14 |
| Small text and its `sub` together | 20 |

- The list of runs cannot be empty. The text of each run and `sub` cannot be empty, and cannot have line breaks or other control characters: text on the island is one line.
