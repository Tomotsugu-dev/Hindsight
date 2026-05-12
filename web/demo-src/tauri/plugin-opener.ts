// Mock 替换 @tauri-apps/plugin-opener
//
// Demo 里把"打开外链"映射到浏览器 window.open；
// 打开文件夹 / 文件路径在 web 没意义，直接 console.warn。

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
  // 文件系统路径在 web 没意义
  // eslint-disable-next-line no-console
  console.warn("[demo] openPath() 在 demo 模式下不可用");
}

export async function revealItemInDir(_path: string): Promise<void> {
  // eslint-disable-next-line no-console
  console.warn("[demo] revealItemInDir() 在 demo 模式下不可用");
}
