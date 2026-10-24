# ADR-0017 · Theme files: separate switches for layout and light or dark, and public variables

- **Date**: 2026-10-24
- **Status**: Proposed
- **Related**: PR #176 (plugins.json format 2)

Users can only pick one of four built-in themes, and they cannot share themes. We read themes from the data folder. Each theme is a `theme.json` that picks a layout, picks light or dark, and sets public CSS variables. A theme can also have a `theme.css`.

## Context

- `<html data-theme>` picks a built-in theme. The theme name decides both the layout and whether the colors are dark. In the components, 115 selectors check for `flat`, 35 check for `dark`, and 11 chart components call `useIsDark`.
- The build generates class names like `._item_1jwc4_1`. When a CSS file changes, its class names change.
- Plugin pages run in a separate iframe. Its CSP does not let them load any images or fonts from the internet. The CSP of the main window blocks fonts from the internet. For images, it allows only `raw.githubusercontent.com` (icons in the plugin market) and `cdn-avatars.huggingface.co` (model icons).

## Decision

We **split the theme into three switches on `<html>`, and read theme files from `<data folder>/themes/<id>/`**.

| Attribute | Values | What it decides |
|---|---|---|
| `data-theme` | `default`, `minimal`, `flat`, `dark`, `custom` | The built-in palette |
| `data-layout` | `card`, `flat` | Cards or flat |
| `data-mode` | `light`, `dark` | Light or dark. Components and charts both check it |

- Components check `data-layout` and `data-mode`, not the theme name. The built-in themes look exactly as before.

| Built-in theme | `data-layout` | `data-mode` |
|---|---|---|
| Default, Minimal | `card` | `light` |
| Airy flat | `flat` | `light` |
| Dark | `card` | `dark` |

Where things are stored, and their names:

| What | Where | Name |
|---|---|---|
| Theme files | `<data folder>/themes/<id>/` | `theme.json`, and an optional `theme.css` |
| The chosen theme | localStorage | Key `hindsight.theme` (the existing key). The value is a built-in theme name or `custom:<id>` |
| A copy of the custom theme in use | localStorage | Key `hindsight.theme.custom`. At startup the theme is applied from this copy, without waiting for the backend to read the files. This way a built-in theme does not flash first |

By default, the data folder is `~/Library/Application Support/Hindsight/` on macOS and `%APPDATA%\Hindsight\` on Windows. Users can move it. Plugins are also stored here (`plugins/`).

- A theme has two parts. `theme.json` sets the layout, light or dark, and the variables. `theme.css` is optional and can change anything. Precedence: the variables in `theme.json` win over `theme.css`, and `theme.css` wins over the built-in theme.
- A custom theme starts from the palette of the default theme. With `data-mode="dark"`, it starts from the dark palette instead. Then `tokens` override the palette.
- Only two things are a public contract: the list of public variables, and the `data-part` names for CSS themes. Renaming any of them breaks other people's themes. The list is written only once, in the Rust code. Tests make sure that `tokens.css` and the JSON Schema in the docs match it. The fields and rules are in `docs/themes/`.
- There is only one way to write a theme: `install`. Import calls it, and a later theme market will call it too.

## Alternatives

- **Variables in JSON, with an optional CSS file (chosen)**: The variables give a stable interface and can also draw a preview. CSS handles everything else.
- **JSON only**: The most stable contract, but authors cannot change anything that is not on the list.
- **CSS only (the Obsidian way)**: The most freedom, but a preview needs a screenshot, and a theme breaks when the class names change.
- **Pick a built-in theme as the base, without separate switches**: Components need no changes, but a dark flat theme is not possible.

## Consequences

- **Benefits**: Users can make and share their own themes. A theme market only needs to download the files and call `install`.
- **Costs and risks**: The variable names and `data-part` names can no longer change freely. Selectors other than `data-part` have no compatibility promise: when the UI changes, a CSS theme may break.

## Data, compatibility, security, and privacy

- **Existing data and migration**: No change. `hindsight.theme` only gets one more kind of value, `custom:<id>`. No database change, no migration, and nothing to sync.
- **Mixed versions and rollback**:
  - `theme.json` has a `format` version number. An unknown version is reported as an error.
  - Unknown fields and variables that are not on the list are ignored. So when a newer version adds something, older versions can still use the theme. The ignored variables are listed for the author.
  - Older versions do not read `themes/`. When they find that a custom theme is chosen, they use Minimal. After the user upgrades again, the theme is still there.
- **Irreversible effects**: Deleting a theme deletes its folder.
- **Security and privacy**: Hindsight does not filter `theme.css`, because CSS escapes can get around a text filter (`\75 rl(` is `url(`). CSS cannot run scripts or load fonts from the internet, but it can load images from the two domains above. So a malicious theme can tell the attacker the user's IP address and when the user opens Hindsight. With attribute selectors, it can also send out the contents of element attributes (such as the value of an input box), one character at a time. Whether the attacker can see these requests depends on the access logs of GitHub and HF. We have not checked this. Theme CSS runs in the main window, which has fewer limits than plugin pages, so users should only install themes they trust.
- **Data folder**: Moving the data folder does not move the themes. Plugins work the same way.

## Verification

- For each built-in theme, the hash of every element's computed style is the same before and after the split.
- A test checks that the list, `tokens.css`, and the Schema match.

## Follow-up

- Theme market: put a `themes.json` in hindsight-plugins, and call `install` to install a theme.
- Spacing and font sizes can become public only after components stop hard-coding these values.
- Plugin pages do not change with custom themes. They do not follow the theme today either. Whether to pass the theme variables to plugins will be decided later.
