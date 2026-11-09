# Plugin permissions

Overview: [Plugin backend architecture](plugins-architecture.md).

**This doc designs**: which permissions a plugin can ask for, and what each one lets it see and do. It covers three things:

- how permissions are named and how their risk levels are set;
- all possible permissions, and which ones are done first;
- what no permission can give.

## Rules

- A name is `<area>.<action>`, such as `stats.read`. An area can have one more level, such as `categories.app.read`. An event permission has the same name as its event, such as `focus.changed`. The click event of island buttons, `island.clicked`, needs no permission of its own and comes with `island`: the event holds only the button `id` the plugin picked. The plugin window event `window.changed` needs no permission either: it goes only to this plugin, and holds only the state of its own window, `{ "state": "open" | "minimized" | "closed" }`.
- Some permissions only add fields to another permission. For example, `focus.app` only adds the app name to `focus.changed` events. Such a permission must be requested together with the one it depends on. Requesting it alone fails the manifest check.
- A permission gives only the fields it lists. Data returned to plugins uses separate structs (see [③ Requesting data](api.md)).
- Reading and writing the plugin's own database needs no permission. But the plugin must ask for disk space with `storage_mb` in its manifest, and can use the database only after the user approves it on the install page (see [Plugin data](data.md)).
- Risk levels:
  - **Low**: only times or totals; it does not show what the user actually did;
  - **Medium**: shows when the user used which app;
  - **High**: shows which websites the user used, shows actual content (window titles, URLs, text from screenshots, AI reports), or can change Hindsight's data, or can reach the network.
  - Websites are high risk: a domain can show the user's interests and situation, such as a hospital or a job site. A subdomain may hold a company name or a user name. Hindsight stores only the domain, not the path or parameters of a URL.
- The install confirmation page marks high-risk permissions separately. When a plugin asks for both a permission that reads data and `network`, it adds a warning: data may be sent off this machine.

## All permissions

Status:

- ✅: done, or being done
- **Later**: done when a plugin needs it

### Reading statistics

| Permission | What it allows | Risk | Status |
|---|---|---|---|
| `stats.read` | Read how long each category and each app was used per day and per hour. Results include the names, colors and icons of the categories and their super-categories. No window titles, URLs or websites. Must be requested with one of the four ranges below | Low | ✅ |
| `stats.today` | Read today only | Low | ✅ |
| `stats.week` | Read this week: Monday to today | Low | ✅ |
| `stats.month` | Read this month: the 1st to today, and this week too | Low | ✅ |
| `stats.history` | Read every day | Low | ✅ |
| `devices.read` | Count statistics by the devices selected in the main window: all devices, or one of them. Without it, only this computer counts | Low | ✅ |
| `categories.read` | List the names, colors and icons of all categories and of their super-categories, without how long they were used | Low | ✅ |
| `super_categories.read` | List the names, colors and icons of all super-categories | Low | ✅ |
| `stats.sites` | Read how long each website was used, such as github.com for 2 hours. No full URLs | High | Later |

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
| `categories.app.read` | List the apps the user used and the category of each, without how long they were used. Must be requested with `categories.read` | Medium | ✅ |
| `categories.app.write` | Change the category of an app, or remove it. This changes the statistics, past days included, and syncs to other devices. Must be requested with `categories.app.read` | High | ✅ |
| `categories.site.read` | List the websites the user used and the websites that have a rule, the rule of each, and the parent domain a rule comes from, without how long they were used. Must be requested with `categories.read` | High | ✅ |
| `categories.site.write` | Set a rule for a website, or remove it. This changes the statistics, past days included, and syncs to other devices. Must be requested with `categories.site.read` | High | ✅ |

- Apps and websites are requested separately: a plugin that only sorts websites cannot see which apps the user used, and the other way around.
- A write permission must be requested with its read permission: the plugin must see the current settings first, so it does not overwrite the ones the user set.
- The lists hold every app and website Hindsight has recorded, not only the ones since the plugin was installed.
- Apps and websites in the Private category are not listed, and a plugin cannot change them: a change fails if the app or website is private before or after it. This is the same as `focus.changed`: a plugin cannot see anything private.
- Excluded from stats works like any other category: a plugin can see what is in it, and can put apps and websites into it or take them out.
- Changing the category of an app changes its whole app group, so its names on other systems change too. If the app is a browser, websites without a rule change too.

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
| `focus.changed` | Get an event when the foreground app or browser URL changes, or when the user goes away. The event has only the time, and whether the user went away. Without this permission, a plugin gets no focus events | Low | ✅ |
| `focus.app` | Add the app name to the event. Must be requested with `focus.changed` | Medium | ✅ |
| `focus.category` | Add the category to the event. Must be requested with `focus.changed` | Low | ✅ |
| `focus.title` | Add the window title and website domain to the event. Must be requested with `focus.changed` | High | ✅ |

### System features

| Permission | What it allows | Risk | Status |
|---|---|---|---|
| `notifications` | Send system notifications | Low | ✅ |
| `notifications.sound` | Play the system alert sound with a notification. Must be requested with `notifications` | Low | ✅ |
| `island` | Put content on the island in the middle of the menu bar (see [The island](island.md)), and get clicks on its own buttons there | Low | ✅ |
| `network` | Connect to the addresses listed in the manifest, such as a server for a friends' leaderboard | High | Later |
| `links.open` | Open a URL in the browser | Low | Later |
| `clipboard.write` | Write text to the clipboard, such as copying a weekly report | Low | Later |

- `network`: the manifest also lists `network_hosts`, such as `["api.example.com"]`, and the install confirmation page shows these addresses to the user. Rust sends the requests and allows only these addresses. A plugin cannot send requests itself: the CSP comes from the plugin file reader and is the same for all plugins, so it cannot allow different addresses for different plugins (see [② Running plugins](runtime.md)).
