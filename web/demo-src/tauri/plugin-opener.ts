// Mock 替换 @tauri-apps/plugin-opener
//
// Demo 里把"打开外链"映射到浏览器 window.open；
// 打开文件夹 / 文件路径在 web 做不到，告诉访客要用桌面版。

import { sayDesktopOnly } from "../desktop-only";

export async function openUrl(url: string): Promise<void> {
  // 真用户可能想点 GitHub 仓库链接、文档链接——给他们打开
  try {
    window.open(url, "_blank", "noopener,noreferrer");
  } catch {
    // 如果是 iframe 内被父页拦截，至少 console 留痕
    // eslint-disable-next-line no-console
    console.warn("[demo] openUrl failed:", url);
  }
}

export async function openPath(_path: string): Promise<void> {
  sayDesktopOnly();
}

export async function revealItemInDir(_path: string): Promise<void> {
  sayDesktopOnly();
}
