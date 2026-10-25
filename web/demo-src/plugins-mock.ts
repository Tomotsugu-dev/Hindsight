// demo 的插件市场：列出 hindsight-plugins 的 plugins.json 里的两个插件，能装、启用、停用、卸载。
// 装了什么只记在内存里，刷新页面就复原。插件本身在 demo 里跑不起来，所以装上后没有页面，
// 侧栏里也不出现。
import { emit } from "./tauri/event";
import type { InstalledPlugin, Market, OnlinePlugin } from "@app/api/plugins";

// 照 plugins.json 抄的，图标是官方仓库 icons/ 里的两张（放在 public/icons/plugins/）。
// 下载次数不写死：demo 不会跟着更新，写了会过时
const ONLINE: OnlinePlugin[] = [
  {
    id: "hindsight-focus",
    name: "Focus",
    author: "Tomotsugu-dev",
    description: {
      "zh-CN": "按工作节奏计时，摸鱼时提醒，支持灵动岛",
      "zh-TW": "按工作節奏計時，摸魚時提醒，支援靈動島",
      en: "Times your work to a rhythm, notifies you when you get distracted, and works with the island",
      ja: "作業のリズムに合わせて時間を計り、サボっているときに知らせます。アイランドに対応",
      "pt-BR": "Cronometra seu trabalho no seu ritmo, avisa quando você se distrai e funciona com a ilha",
      es: "Cronometra tu trabajo a tu ritmo, te avisa cuando te distraes y funciona con la isla",
    },
    repo: "Tomotsugu-dev/hindsight-focus",
    versions: [
      {
        version: "0.2.1",
        url: "https://github.com/Tomotsugu-dev/hindsight-focus/releases/download/0.2.1/hindsight-focus-0.2.1.zip",
        sha256: "0459a781334a1a3fe31dfd0641bde73d96f9782970a1b405c1684363d7fe539c",
        permissions: [
          "stats.read",
          "stats.today",
          "stats.month",
          "categories.read",
          "super_categories.read",
          "focus.changed",
          "focus.app",
          "focus.category",
          "notifications",
          "island",
        ],
        storage_mb: 1,
        kind: "new",
      },
    ],
    icon_url: "/demo/icons/plugins/hindsight-focus.png",
    official: true,
    download_count: null,
  },
  {
    id: "hindsight-reminder",
    name: "网站与程序提醒",
    author: "Yoshino2000",
    description: {
      "zh-CN": "进入指定网站或程序时发送自定义通知，支持多规则、启停和冷却时间",
      en: "Custom notifications when entering a website or app, with multiple rules, pause switches and cooldowns",
    },
    repo: "Yoshino2000/hindsight-reminder",
    versions: [
      {
        version: "1.0.1",
        url: "https://github.com/Yoshino2000/hindsight-reminder/releases/download/1.0.1/hindsight-reminder-1.0.1.zip",
        sha256: "5052bed59b1af9f4fd59ee34dbbeb4f14b534a559680dbd5d709e70b2a05e197",
        permissions: ["focus.changed", "focus.app", "focus.title", "notifications"],
        storage_mb: 1,
        kind: "new",
      },
    ],
    icon_url: "/demo/icons/plugins/hindsight-reminder.png",
    official: false,
    download_count: null,
  },
];

const installed = new Map<string, InstalledPlugin>();

/** 每个插件只有一个版本：装了就是「当前版本」，没装就是「新装」。 */
function withKind(plugin: OnlinePlugin): OnlinePlugin {
  const kind = installed.has(plugin.id) ? "same" : "new";
  return { ...plugin, versions: plugin.versions.map((v) => ({ ...v, kind })) };
}

// 应用里装、卸、启停插件后，后端发这个事件，侧栏和插件页据此重读图标
function changed(): void {
  void emit("plugin-pages-changed", []);
}

/** demo 能回答的插件命令：命令名 → 处理函数。参数名和 src/api/plugins.ts 里的一样。 */
export const pluginCommands: Record<string, (args: Record<string, unknown>) => unknown> = {
  plugin_open_market: (): Market => ({
    installed: [...installed.values()],
    installedOnline: ONLINE.filter((p) => installed.has(p.id)).map(withKind),
    downloadable: ONLINE.filter((p) => !installed.has(p.id)).map(withKind),
    localZips: [],
    onlineError: null,
  }),
  plugin_install_online: ({ id, approved, storageMb }) => {
    const plugin = ONLINE.find((p) => p.id === id);
    if (!plugin) throw new Error(`no plugin ${String(id)}`);
    installed.set(plugin.id, {
      id: plugin.id,
      name: plugin.name,
      version: plugin.versions[0].version,
      enabled: true,
      permissions: approved as string[],
      storageMb: storageMb as number | null,
      ui: { hasPage: false, opensIn: null, windowX: null, windowY: null, windowWidth: 0, windowHeight: 0 },
      runsInBackground: false,
      installedAt: new Date().toISOString(),
    });
    changed();
  },
  plugin_uninstall: ({ id }) => {
    installed.delete(id as string);
    changed();
  },
  plugin_set_enabled: ({ id, enabled }) => {
    const plugin = installed.get(id as string);
    if (plugin) installed.set(plugin.id, { ...plugin, enabled: enabled as boolean });
    changed();
  },
  plugin_icons: () =>
    Object.fromEntries(
      [...installed.keys()].map((id) => [id, ONLINE.find((p) => p.id === id)?.icon_url ?? ""]),
    ),
  plugin_pages: () => [],
};
