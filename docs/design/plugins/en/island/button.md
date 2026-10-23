# button: a button

![button](../../images/island/button-en.svg)

Only in the open island. The left button has only an icon. The right one has an icon and text.

```json
{ "type": "button", "id": "pause", "icon": "Pause" }
```

```json
{ "type": "button", "id": "skip", "icon": "SkipForward", "text": "Skip" }
```

| Field | Required | Notes |
|---|---|---|
| `id` | Yes | A name the plugin picks. It is handed back to the plugin as it is when the button is clicked |
| `icon` | Yes | The icon name, the same as `name` in [`icon`](icon.md) |
| `text` | No | The text on the button |

- A click on a button does not open the page. The plugin's background script gets `island.clicked` with `{ "button": "pause" }`. If a plugin that does not run in the background puts a button on the island, `island.show` fails: the plugin could not get the click.

## Allowed places

- `open.left`, `open.right`. Buttons do not fit in the closed island.

## Length limits

- `id` is at most 64 bytes.
- `text` is at most 20 wide. For how width is measured, see "Space in each place" in [The island](../island.md). It cannot be empty, and cannot have line breaks or other control characters.
