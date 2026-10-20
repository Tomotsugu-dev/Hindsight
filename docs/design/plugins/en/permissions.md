# Plugin permissions

Overview: [Plugin backend architecture](plugins-architecture.md).

**This doc designs**: which permissions a plugin can ask for, and what each one lets it see and do. It covers three things:

- how permissions are named and how their risk levels are set;
- all possible permissions, and which ones the MVP does first;
- what no permission can give.

## Rules

- A name is `<area>.<action>`, such as `stats.read`. An event permission has the same name as its event, such as `focus.changed`.
- Some permissions only add fields to another permission. For example, `focus.app` only adds the app name to `focus.changed` events. Such a permission must be requested together with the one it depends on. Requesting it alone fails the manifest check.
- A permission gives only the fields it lists. Data returned to plugins uses separate structs (see [③ Requesting data](api.md)).
- Reading and writing the plugin's own database needs no permission. But the plugin must ask for disk space with `storage_mb` in its manifest, and can use the database only after the user approves it on the install page (see [Plugin data](data.md)).
- Risk levels:
  - **Low**: only times or totals; it does not show what the user actually did;
  - **Medium**: shows when the user used which app or website;
  - **High**: shows actual content (window titles, URLs, text from screenshots, AI reports), or can change Hindsight's data, or can reach the network.
- The install confirmation page marks high-risk permissions separately. When a plugin asks for both a permission that reads data and `network`, it adds a warning: data may be sent off this machine.

## All permissions

Status:

- **MVP**: done in the first version
- **Later**: done when a plugin needs it

### Reading statistics

| Permission | What it allows | Risk | Status |
|---|---|---|---|
| `stats.read` | Read how long each category and each app was used per day and per hour. Results include the names, colors and icons of the categories and their super-categories. No window titles, URLs or websites. Must be requested with one of the four ranges below | Low | MVP |
| `stats.today` | Read today only | Low | MVP |
| `stats.week` | Read this week: Monday to today | Low | MVP |
| `stats.month` | Read this month: the 1st to today, and this week too | Low | MVP |
| `stats.history` | Read every day | Low | MVP |
| `devices.read` | Count statistics by the devices selected in the main window: all devices, or one of them. Without it, only this computer counts | Low | MVP |
| `categories.read` | List the names, colors and icons of all categories and of their super-categories, without how long they were used | Low | MVP |
| `super_categories.read` | List the names, colors and icons of all super-categories | Low | MVP |
| `stats.sites` | Read how long each website was used, such as github.com for 2 hours. No full URLs | Medium | Later |

- Each range includes the narrower ones: a plugin with a wider range can also read every day a narrower range can.
- Requesting `stats.read` without a range fails the manifest check. So the install confirmation page always shows the user how far back the plugin can read.

### Reading details

| Permission | What it allows | Risk | Status |
|---|---|---|---|
| `activity.read` | Read each usage record: start and end time, app name, category | Medium | Later |
| `activity.titles` | Add window titles and URLs to the `activity.read` records | High | Later |

### Reading category settings

| Permission | What it allows | Risk | Status |
|---|---|---|---|
| `categories.read` | Read the category list (names, colors), and which category each app and website belongs to | Low | Later |
| `categories.write` | Change categories, and which category apps and websites belong to. This changes the statistics and syncs to other devices | High | Later |

### Screen memory and AI

| Permission | What it allows | Risk | Status |
|---|---|---|---|
| `screen.text` | Search the text recognized in screenshots. Results include the time and app name, but not the screenshot itself | High | Later |
| `screen.images` | Read screenshots | High | Later |
| `ai.summaries.read` | Read the text of AI daily and weekly reports | High | Later |
| `ai.generate` | Generate text with the AI model the user set up. This uses the user's computing power or API quota | Medium | Later |
| `chat.read` | Read the user's chats with the AI | High | Later |

### Writing data

| Permission | What it allows | Risk | Status |
|---|---|---|---|
| `activity.write` | Add records to the timeline, such as data imported from other apps or a meeting away from the computer. They count in the statistics and sync to other devices | High | Later |

### Events

The plugin's background script receives events pushed by Hindsight (see [④ Receiving events](events.md)).

| Permission | What it allows | Risk | Status |
|---|---|---|---|
| `focus.changed` | Get an event when the foreground app or browser URL changes, or when the user goes away. The event has only the time, and whether the user went away. Without this permission, a plugin gets no focus events | Low | MVP |
| `focus.app` | Add the app name to the event. Must be requested with `focus.changed` | Medium | MVP |
| `focus.category` | Add the category to the event. Must be requested with `focus.changed` | Low | MVP |
| `focus.title` | Add the window title and website domain to the event. Must be requested with `focus.changed` | High | MVP |

### System features

| Permission | What it allows | Risk | Status |
|---|---|---|---|
| `notifications` | Send system notifications | Low | MVP |
| `network` | Connect to the addresses listed in the manifest, such as a server for a friends' leaderboard | High | Later |
| `links.open` | Open a URL in the browser | Low | Later |
| `clipboard.write` | Write text to the clipboard, such as copying a weekly report | Low | Later |

- `network`: the manifest also lists `network_hosts`, such as `["api.example.com"]`, and the install confirmation page shows these addresses to the user. Rust sends the requests and allows only these addresses. A plugin cannot send requests itself: the CSP comes from the plugin file reader and is the same for all plugins, so it cannot allow different addresses for different plugins (see [② Running plugins](runtime.md)).
