# Hindsight themes

A theme changes how Hindsight looks. It is a folder with two files:

| File | Required | What it does |
|---|---|---|
| `theme.json` | Yes | Picks the layout and light or dark, and sets public CSS variables |
| `theme.css` | No | Any other CSS. It can change anything in the main window |

[`theme.schema.json`](theme.schema.json) describes `theme.json`. [`example/`](example/) is a complete theme.

## Install a theme

In Hindsight, open **Settings → Appearance → Custom themes**.

- **Get themes**: opens the [theme website](https://hindsight.kyosweb.com/themes/) in your browser. Download a theme there and unzip it, then import it.
- **Import theme**: pick the theme's `theme.json`. If the same folder has a `theme.css`, it is imported too. A theme with the same id is replaced.
- **Open themes folder**: copy the theme's folder into it. The folder name must be the theme's id.

The themes folder is `themes/` in the data folder. By default this is `~/Library/Application Support/Hindsight/themes/` on macOS and `%APPDATA%\Hindsight\themes\` on Windows.

When you switch back to Hindsight, it reads the themes folder again. If you edit the theme that is in use, the window shows your changes then. This is the fastest way to work on a theme: edit the files in the themes folder, then switch back to Hindsight.

A theme that cannot be read is still listed, with the reason. If the theme in use breaks or is removed, Hindsight shows Minimal until the theme is fixed.

## theme.json

```json
{
  "$schema": "https://raw.githubusercontent.com/Tomotsugu-dev/Hindsight/main/docs/design/themes/theme.schema.json",
  "format": 1,
  "id": "harbor",
  "name": "Harbor",
  "version": "1.0.0",
  "author": "Hindsight",
  "description": { "en": "Slate blue on cool grey.", "zh-CN": "冷灰底色，石板蓝点缀。" },
  "layout": "card",
  "mode": "light",
  "tokens": {
    "--color-accent": "#3b6e8f",
    "--color-accent-soft": "rgba(59, 110, 143, 0.12)"
  }
}
```

| Field | Required | Rule |
|---|---|---|
| `format` | Yes | `1` |
| `id` | Yes | Lowercase letters, digits and `-`. 1 to 63 characters, and not starting or ending with `-`. It must be the folder name |
| `name` | Yes | At most 64 characters, and not only spaces |
| `version` | Yes | Three numbers, for example `1.0.0` |
| `author` | No | At most 64 characters |
| `description` | No | Language code → text, for example `"en"`, `"zh-CN"`, `"zh-TW"`, `"ja"`, `"es"`, `"pt-BR"`. Hindsight shows the interface language, then `en`, then the first language by code |
| `layout` | No | `card` (default) or `flat`. `flat` removes the shadows of cards, and has smaller corners and lighter text |
| `mode` | No | `light` (default) or `dark`. Charts follow it |
| `tokens` | No | Public variable → CSS value. See below |
| `$schema` | No | Lets your editor check the file. Hindsight ignores it |

Hindsight ignores fields it does not know. A newer version of Hindsight can add fields, and older versions can still use the theme.

### Public variables

A theme starts from the colors of the Default theme. With `"mode": "dark"`, it starts from the Dark theme's colors. `tokens` then changes the variables it names. The default values are in [`src/styles/tokens.css`](../../../src/styles/tokens.css).

| Group | Variables |
|---|---|
| Accent | `--color-accent`, `--color-accent-soft` (the light accent background, such as the selected item) |
| Sidebar | `--sidebar-gradient` (any CSS background), `--sidebar-border` |
| Surfaces | `--surface-app` (the page), `--surface-card`, `--surface-card-elevated`, `--surface-overlay` (menus and dialogs), `--surface-1`, `--surface-2` |
| Selected item in the sidebar | `--pill-bg`, `--pill-blur` (a `backdrop-filter` value), `--pill-shadow`, `--text-on-pill` |
| Hover | `--hover-bg` |
| Text | `--text-strong`, `--text-regular`, `--text-muted`, `--text-faint` |
| Status | `--status-ok`, `--status-pending`, `--status-idle`, `--status-error`, `--status-ok-soft`, `--status-error-soft`, `--status-pending-soft` |
| Borders | `--border-subtle`, `--border-default`, `--border-strong`, `--hairline-hi` (the thin light line on the top edge of cards and buttons) |
| Controls | `--control-bg`, `--control-bg-hover` (menus and date buttons) |
| Scrollbar | `--scrollbar-thumb`, `--scrollbar-thumb-hover` |
| Shadows | `--shadow-sm`, `--shadow-md`, `--shadow-lg`, `--shadow-inset-hi`, `--shadow-strength` |
| Corners | `--radius-xs`, `--radius-sm`, `--radius-md`, `--radius-lg`, `--radius-xl` |
| Fonts | `--font-sans`, `--font-mono` (paths and code) |

- `--shadow-strength` is a number. The opacity of every dark shadow is multiplied by it: `0` removes the shadows, `1` is the default, and `2` makes them twice as dark.
- Set `--color-accent-soft` together with `--color-accent`. The light accent background does not change by itself.
- A value can have at most 300 characters. It cannot be only spaces, and it cannot have `;`, `{`, `}` or control characters.
- A variable that is not on this list has no effect. The theme still works, and its card in Settings lists the variables that had no effect.

## theme.css

`theme.css` comes after all of Hindsight's own CSS, so a rule in it wins over a built-in rule with the same specificity. The variables in `theme.json` win over `theme.css`.

Use these names in selectors. They do not change between versions:

| Selector | What it is |
|---|---|
| `[data-part="app"]` | The whole window |
| `[data-part="sidebar"]` | The sidebar |
| `[data-part="nav-item"]` | An item in the sidebar. The current page has `aria-current="page"` |
| `[data-part="content"]` | The area right of the sidebar |
| `[data-part="page-header"]` | The title at the top of a page |
| `[data-part="tabs"]` | A row of tabs |
| `[data-part="card"]` | A card on the report pages |
| `[data-part="chart"]` | A chart |
| `[data-part="ranked-list"]` | A ranked list of apps |
| `[data-part="drawer"]` | The panel with an app's details |
| `[data-part="dialog"]` | A dialog |
| `[data-part="section"]` | A section on a settings page |
| `[data-part="row"]` | A row in a section |
| `[data-part="toggle"]` | A switch. It has `aria-checked="true"` when it is on |
| `[data-part="slider"]` | A slider |

`<html>` also has `data-layout` (`card` or `flat`) and `data-mode` (`light` or `dark`), from `theme.json`.

Do not use class names. They look like `._item_1jwc4_1` and change with every version. Other selectors, such as element names inside a part, can also break when the page changes.

Limits of the main window:

- `@import` is ignored. Put all the CSS in `theme.css`.
- Fonts from the internet are blocked. Images from the internet are blocked too, except from two hosts that Hindsight uses for icons. Put a font or an image in `theme.css` as a `data:` URL, or use a font that is installed on the computer.
- `theme.json` can be at most 64 KB, and `theme.css` at most 512 KB. Both must be UTF-8.

## Safety

`theme.css` runs in the main window. It can hide or change anything there. Install only themes you trust, and do not load anything from the internet in a theme you share.

## Compatibility

The public variables and the `data-part` names are a promise: Hindsight keeps their names, so themes that use them keep working. Everything else on the page can change in any version.

## Share a theme

To list your theme on the [theme website](https://hindsight.kyosweb.com/themes/), see [Themes](https://github.com/Tomotsugu-dev/hindsight-plugins/blob/main/CONTRIBUTING.md#themes) in the hindsight-plugins repository.
