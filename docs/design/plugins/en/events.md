# ④ Receiving events

Overview: [Plugin backend architecture](plugins-architecture.md).

**This doc designs**: how the app pushes focus changes to plugins. It covers three things:

- when capture sees a focus change, how it tells the event dispatcher without slowing capture down;
- which fields each plugin gets, based on its permissions;
- when something fails, how to limit the damage to one plugin and one event.

Example: notify the focus plugin when the user switches to YouTube.

```mermaid
sequenceDiagram
  participant C as capture/
  participant R as repo/
  box rgba(148,163,184,0.15) Not built yet
    participant E as Event dispatcher
    participant H as Forwarder<br/>in the plugin<br/>background window
  end
  participant P as Background script
  C->>E: Focus changed<br/>Chrome, youtube.com
  Note over E: Goes into its<br/>own channel.<br/>Capture does not wait.
  E->>R: Which enabled plugins<br/>have focus.changed?
  R-->>E: hindsight-focus<br/>and its permissions
  Note over E: If none,<br/>drop this event
  E->>R: Get the category<br/>(only if a plugin<br/>has focus.category)
  R-->>E: fun
  E->>E: Build the event<br/>by permission:<br/>time and category only
  E->>H: emit_to<br/>plugin id + event
  H->>P: postMessage<br/>10:30, fun
```

- **Why the event dispatcher has its own channel**: the capture loop does not need to know that plugins exist, and it does not wait for them. Without the channel:
  - `capture/` would call `plugins/` directly;
  - the time to look up categories and permissions would count toward the capture that runs every 5 seconds;
  - an error on the plugin side would also slow down capture.
- Capture gains only one line: when the focus really changes, put one item into the event dispatcher's channel.
  - Not where a new session starts. Capture also starts new sessions on a timer for screenshots, and then the focus has not changed ([service.rs:552-568](../../../../src-tauri/src/capture/service.rs#L552-L568)).
  - Do not wait when putting the item in.
  - If the dispatcher falls behind, drop the older items. Only the latest focus event matters.
- Events are built by permission in Rust. Events for plugins use a separate struct that holds only the fields the plugin may see. If internal code adds a field later, it is not pushed out unless it is added to this struct.
- An error in one plugin affects only that plugin. Each plugin gets its own push: if the push to A fails, log it and go on to B.
- The event dispatcher must not panic. Release builds set `panic = "abort"` ([Cargo.toml:136](../../../../src-tauri/Cargo.toml#L136)), so one panic quits the whole app. Failures in the plugin lookup, the category lookup or the push all return errors. The dispatcher drops this event and waits for the next focus change.
