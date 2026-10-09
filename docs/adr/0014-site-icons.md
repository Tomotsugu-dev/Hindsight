# ADR-0014 · Website icons: downloaded from the website, stored on this device

- **Date**: 2026-10-09
- **Status**: Proposed
- **Related**: issue #31 · [ADR-0013](0013-site-category-rules.md)

The Websites tab shows only addresses, so users have to read each one to recognize a website. With this proposal, once the user turns on a switch, Hindsight requests the icon from the website itself and stores it on this device. Icons are not synced. The switch is off by default.

## Flow

```mermaid
sequenceDiagram
    participant P as Websites tab
    participant B as Backend
    participant W as Website

    P->>B: List websites
    B-->>P: Website list<br/>(with stored icons)
    Note over B: Switch off: stop here
    B->>B: Skip local addresses,<br/>pick websites with no icon<br/>(including failures<br/>older than 7 days)
    B->>W: GET /favicon.ico
    alt Icon received
        W-->>B: ICO / PNG / JPEG
    else No icon
        B->>W: GET / (home page)
        W-->>B: Icon address<br/>in the page
        B->>W: GET icon address
        W-->>B: Icon or failure
    end
    B->>B: Convert to 32×32 PNG,<br/>save as a file;<br/>leave a .failed file<br/>on failure
    B-->>P: Newly downloaded icons
```

| Case | Result |
|---|---|
| Switch off | No requests. Icons already downloaded still show. |
| `localhost`, IP addresses, names ending in `.local`, host names without a dot | Skipped, no request |
| `<domain>.png` exists | Shown as is. The website is never requested again. |
| Download succeeds | Saved as `<domain>.png` |
| Download fails or the format is not supported (such as SVG) | An empty `<domain>.failed` file is left, and the website is not tried again for 7 days. The page shows the globe icon. |
| Clear data | The whole `site-icons/` folder is deleted, together with the activity records |

## Decisions and rationale

1. **Download from the website itself.** All browsers use the same path, and no browser's icon cache needs handling. The cost is that Hindsight goes online on its own.
2. **The switch is off by default and explains which websites will be contacted when turned on.** Without it, there are no new network requests, the same as before. The explanation states that company intranet addresses that look like normal domains cannot be recognized and will be contacted too.
3. **Skip local addresses.** Icons of local development servers and devices on the local network are of no use, and requesting them can have side effects.
4. **Store icons as files, not in the database, and do not sync them.** They go in `site-icons/` in the folder that holds the database, next to the `icons/` folder of app icons. A success is saved as `<domain>.png`; a failure leaves an empty `<domain>.failed`, whose modification time is the time of the failure. The frontend gets the file path and shows the file directly, like app icons, so images are not turned into base64 text and passed over IPC. Each device downloads its own icons once the switch is on. The sync format does not change.
5. **Once an icon is downloaded, the website is not requested again.** Websites rarely change their icons. No download time is kept, so there is no periodic refresh.
6. **Support only ICO, PNG, and JPEG.** SVG would need another large dependency; these websites keep the globe icon.
7. **When `/favicon.ico` fails, read the icon address declared in the home page.** In a test of 20 common websites, 4 did not serve `/favicon.ico`: 2 had no such file (404), 1 redirected to a sign-in page (returned HTML), and 1 blocked the request as a bot (403). Reading the home page is expected to recover the first 3 (not tested); the blocked one cannot be recovered. Whichever step returns the icon, the content is checked to be an image; the status code alone is not trusted.

## Alternatives

| Option | Reason not chosen |
|---|---|
| Read each browser's own icon cache | Needs the file location of every browser on macOS and Windows, and handling files the browser keeps locked. Safari's cache needs Full Disk Access. |
| Use a third-party icon service | Sends every domain the user visited to a third party |
| Store icons in the database | The frontend would get images as base64 text over IPC, an extra copy in WebView memory; it also needs a migration |
| Sync icons through the cloud | Changes the sync format; every device can download icons on its own |

## Costs, data, and compatibility

- **Privacy**: The website receives a request from this device and sees its IP address, as with a visit in a browser. The difference is that the request is sent in the background, at a time unrelated to browsing.
- **Network**: A website is not requested again after its icon is downloaded. A failed website is retried at most once every 7 days.
- **Storage**: No database change and no migration. One icon takes 1–2 KB; a thousand websites take about 2–3 MB.
- **Rollback**: Older versions do not read this folder and leave it alone. After upgrading again, the icons are still there.

## Verification

- No requests are sent while the switch is off.
- Local addresses are not requested.
- A website with a `.png` is not requested again.
- When `favicon.ico` fails, the icon address is read from the home page. After both fail, the website is not tried again for 7 days.
- ICO, PNG, and JPEG are converted to 32×32 PNG; SVG is recorded as a failure.
- After Clear data, the `site-icons/` folder is gone.
