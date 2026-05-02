export interface Category {
  id: string;
  name: string;
  color: string;
  apps: string[];
  builtin: boolean;
}

export const DEFAULT_CATEGORIES: Category[] = [
  {
    id: "code",
    name: "编程",
    color: "#a78bfa", // violet-400
    builtin: true,
    apps: ["code.exe", "idea64.exe", "pycharm64.exe", "WebStorm.exe"],
  },
  {
    id: "browse",
    name: "浏览",
    color: "#60a5fa", // blue-400
    builtin: true,
    apps: ["chrome.exe", "firefox.exe", "msedge.exe"],
  },
  {
    id: "talk",
    name: "沟通",
    color: "#34d399", // emerald-400
    builtin: true,
    apps: ["WeChat.exe", "DingTalk.exe", "Lark.exe", "slack.exe"],
  },
  {
    id: "design",
    name: "设计",
    color: "#fbbf24", // amber-400
    builtin: true,
    apps: ["Figma.exe", "Photoshop.exe", "Illustrator.exe"],
  },
  {
    id: "fun",
    name: "娱乐",
    color: "#fb7185", // rose-400
    builtin: true,
    apps: ["Spotify.exe", "Steam.exe", "网易云音乐.exe"],
  },
  {
    id: "other",
    name: "其他",
    color: "#94a3b8", // slate-400
    builtin: true,
    apps: [],
  },
];

export function getCategory(id: string): Category | undefined {
  return DEFAULT_CATEGORIES.find((c) => c.id === id);
}
