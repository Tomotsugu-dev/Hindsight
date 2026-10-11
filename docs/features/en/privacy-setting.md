# Privacy Settings

- [Overview](#overview)
- [Privacy Filters](#privacy-filters)
- [Defaults](#defaults)

## Overview

Exclude selected apps, windows, and web pages from screenshots in “Settings → Privacy”.

## Privacy Filters

Add keywords to prevent matching windows and web pages from being captured in screenshots.

- **App filter**: Skips screenshots when an app name or window title contains a keyword.
- **Browser filter**: Skips screenshots when the URL in the browser's address bar contains a keyword.
- **Record website domains**: Records the website you visit, such as `github.com`, rather than the specific page. For example, opening `https://github.com/xxx/issues/1` records only `github.com`.

## Defaults

| Setting | Default | Description |
| --- | --- | --- |
| Enable screenshots | Off | Enable in “Settings → General” |
| Browser filter | 10 login-related keywords | Includes `/login`, `/signin`, `/auth`, `/oauth`, and `/password`; editable |
| App filter | Empty | - |
| Record website domains | On | - |
