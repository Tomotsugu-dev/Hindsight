# ② Running plugins

Overview: [Plugin backend architecture](plugins-architecture.md).

**This doc designs**: how installed plugins start running. It covers two things:

- at startup, how the app opens the plugin background window and loads the plugins that run in the background;
- when the WebView asks for a plugin's file, how the plugin file reader hands it over.

```mermaid
sequenceDiagram
  participant S as Startup
  participant M as Background<br/>window manager
  participant R as repo/
  participant H as Forwarder<br/>in the plugin<br/>background window
  participant F as Plugin file reader
  participant D as Plugin library
  S->>M: After migrations<br/>and capture start
  M->>R: Which enabled plugins<br/>run in the background?
  R-->>M: hindsight-focus
  Note over M: If none do,<br/>do not open the window.<br/>This saves a WebView.
  M->>H: Create the plugin<br/>background window
  H->>M: invoke<br/>get the plugin list
  M-->>H: hindsight-focus<br/>0.1.0
  H->>H: One sandboxed<br/>iframe per plugin
  H->>F: hindsight-plugin://<br/>hindsight-focus/0.1.0/<br/>background.js
  F->>D: Read hindsight-focus/<br/>0.1.0/background.js
  D-->>F: Script file
  F-->>H: Script file<br/>+ CSP
```

> Content Security Policy (CSP): a rule sent along with a web page. It tells the WebView what the page may and may not do, for example no network access. If plugin JS tries to send data to an outside website, the WebView itself blocks the request.

## Implementation details

- If no plugin runs in the background, do not open the plugin background window. Users who use no plugins do not pay the memory cost.
- When creating the window, set `background_throttling` to `Disabled`. Otherwise plugin timers stop while the window is hidden. Windows does not support this setting; test it there separately.
- Keep file paths inside the version folder in the URL (`<id>/<version>/`). Block paths like `../` that climb up.
- Build the plugin file reader as a Tauri custom protocol. Register it once at app startup, next to the existing `.plugin(...)` calls in `lib.rs`.
