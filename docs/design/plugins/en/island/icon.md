# icon: an icon

![icon](../../images/island/icon.svg)

Left: closed. Right: open.

```json
{ "type": "icon", "name": "Code", "color": "#a78bfa" }
```

| Field | Required | Notes |
|---|---|---|
| `name` | Yes | The icon name, see below |
| `color` | No | `#rrggbb`. White if left out |

## Allowed places

| Place | Size |
|---|---|
| `closed.left`, `closed.right` | 14 px. A side with only an icon is round |
| `open.left`, `open.right` | 22 px |

## Icon names

- The category icons of the main window, the same as `icon` in the results of `categories.list`: `Code` `Terminal` `Database` `GitBranch` `Bug` `Server` `Cloud` `Bot` `Globe` `Search` `Compass` `Newspaper` `Rss` `MessageCircle` `MessagesSquare` `Mail` `CalendarDays` `Briefcase` `FileText` `Table2` `Presentation` `NotebookPen` `PenLine` `Calculator` `ChartLine` `Brush` `Palette` `PenTool` `Box` `Image` `Camera` `Clapperboard` `Gamepad2` `Music` `Headphones` `Mic` `Film` `Video` `Tv` `GraduationCap` `BookOpen` `Languages` `Heart` `Coffee` `Dumbbell` `ShoppingCart` `Wallet` `Download` `FolderOpen` `HardDrive` `Shield` `Lock` `Activity` `Clock` `Wrench` `Sparkles` `Monitor` `MoreHorizontal` `EyeOff` `Tag`
- For buttons: `Play` `Pause` `Square` `SkipForward` `RotateCcw` `Check` `X` `Plus` `Minus`
- Both `icon` and [`button`](button.md) can use both sets. The names are the same as in the Lucide icon library.
- An unknown name shows as `Tag`, with no error.
- A name is at most 64 bytes.
