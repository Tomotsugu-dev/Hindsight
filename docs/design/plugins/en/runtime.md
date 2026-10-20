# ② Running plugins

Overview: [Plugin backend architecture](plugins-architecture.md).

**This doc designs**: how installed plugins start running. It covers four things:

- at startup, how the app opens the plugin background window and loads the plugins that run in the background;
- when one plugin hangs, how it avoids stopping the other plugins and the app;
- how plugin pages show in the main window and in their own windows;
- when the WebView asks for a plugin's file, how the plugin file reader hands it over.

## What is in the plugin background window

```mermaid
flowchart TB
  subgraph BG["Background window"]
    subgraph MAIN["Main thread, shared"]
      R["Forwarder<br/>Hindsight's JS"]
      L1["iframe: hindsight-focus<br/>launcher"]
      L2["iframe: pomodoro<br/>launcher"]
    end
    W1["Worker thread<br/>background.js of<br/>hindsight-focus"]
    W2["Worker thread<br/>background.js of<br/>pomodoro"]
  end
  L1 -->|"starts"| W1
  L2 -->|"starts"| W2
```

- **One Worker per plugin**: all iframes share one main thread, so if one plugin hangs, all of them stop. A Worker is a separate thread, so a hung Worker can be stopped on its own.
- **One iframe per plugin** (a page inside another page): a Worker can only be started with a script from the same origin. The iframe opens at the plugin's own origin, so storage is separate between plugins and between a plugin and Hindsight, and the Worker uses the CSP from the plugin file reader.
- **Hindsight provides each plugin's launcher**: the launcher runs on the shared main thread, so it cannot hold the author's code. It only starts the Worker, passes messages, and stops the Worker.
- **Heartbeat**: the launcher pings the Worker every 10 seconds, and stops it when two pings in a row get no reply. The plugin background is hidden, so when a plugin is stuck in a loop, the user cannot see which plugin is using the CPU.
- **Not one window per plugin**: each window is one process and uses several times more memory. Tauri also injects its call interface and key into each window's main page.

## At startup

```mermaid
sequenceDiagram
  participant M as Background<br/>window manager
  participant R as repo/
  participant H as Forwarder<br/>in the plugin<br/>background window
  participant L as Launcher<br/>one iframe<br/>per plugin
  participant F as Plugin<br/>file reader
  Note over M: After migrations<br/>and capture start
  M->>R: Which enabled plugins<br/>run in the background?
  R-->>M: hindsight-focus
  Note over M: If none do,<br/>do not open the window
  M->>H: Create the plugin<br/>background window
  H->>M: invoke<br/>get the plugin list
  M-->>H: hindsight-focus<br/>0.1.0
  H->>L: Create an iframe<br/>at hindsight-focus's<br/>own origin
  L->>F: Ask for the launcher
  F-->>L: Hindsight's own<br/>launcher + CSP
  L->>F: new Worker<br/>ask for background.js
  F-->>L: background.js from<br/>the plugin library<br/>+ CSP
  Note over L: background.js runs<br/>in a Worker thread
```

> Content Security Policy (CSP): a rule sent along with a web page. It tells the WebView what the page may and may not do, for example no network access. If plugin JS tries to send data to an outside website, the WebView itself blocks the request.

## Plugin pages

```mermaid
flowchart TB
  subgraph MW["Main window"]
    H1["Hindsight's page<br/>forwarder"]
    P1["iframe: Focus page<br/>plugin's own origin"]
  end
  subgraph PW["Pomodoro's own window"]
    H2["Hindsight's page<br/>forwarder"]
    P2["iframe: Pomodoro page<br/>plugin's own origin"]
  end
  P1 -->|"hindsight.call()"| H1
  P2 -->|"hindsight.call()"| H2
```

- **The plugin page is in an iframe inside a Hindsight page**: the iframe opens at the plugin's own origin, so it cannot reach the call interface Tauri injects into Hindsight's pages, and its storage is separate from Hindsight's. It hands requests to the forwarder in the outer page, and the forwarder tells which plugin it is by which iframe the message came from.
- **The main window and a page's own window show the same plugin page**: only the outer page differs, so the author does not write two versions.

## Implementation details

- If no plugin runs in the background, do not open the plugin background window. Users who use no plugins do not pay the memory cost.
- When creating the window, set `background_throttling` to `Disabled`. Otherwise plugin timers stop while the window is hidden. Windows does not support this setting; test it there separately.
- Put the plugin id in the host name, so pages of different plugins have different origins. End it with `.localhost`: on Windows the URL is rewritten to `http://hindsight-plugin.hindsight-focus.localhost/…`, and only a host ending in `.localhost` counts as a secure context.
- Keep file paths inside the version folder in the URL (`<id>/<version>/`). Block paths like `../` that climb up.
- The plugin background uses four files. Their URLs are all under `hindsight-plugin://<plugin id>.localhost/<version>/`:
  - Hindsight writes three of them in Rust. For these names, the plugin file reader returns them directly and does not read the plugin library:
    1. `_launcher.html`, `_launcher.js`: the launcher (the CSP does not allow inline scripts, so it is split into two files).
    2. `_worker.js`: defines `hindsight.call()`, then loads `background.js`.
  - `background.js`: the script written by the plugin author.
- Build the plugin file reader as a Tauri custom protocol. Register it once at app startup, next to the existing `.plugin(...)` calls in `lib.rs`.
