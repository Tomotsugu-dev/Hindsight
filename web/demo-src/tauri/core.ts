// Mock 替换 @tauri-apps/api/core，给 demo 用
//
// 真 invoke 走 Tauri IPC；demo 里所有数据从 api-mock 走，
// 这里只是兜底——如果某组件绕过 api 直接调用 invoke，我们记一条 warn 不让它崩。

export async function invoke<T = unknown>(
  cmd: string,
  _args?: Record<string, unknown>,
): Promise<T> {
  // eslint-disable-next-line no-console
  console.warn(`[demo] invoke("${cmd}") 在 demo 模式下未实现，返回 undefined`);
  return undefined as T;
}

// AppIcon 用 convertFileSrc 把后端绝对路径转 asset:// URL；demo 里 api-mock 给的已经是
// 现成可用的图标 URL（/demo/icons/*.png 或 simpleicons CDN），原样返回即可。
export function convertFileSrc(filePath: string, _protocol?: string): string {
  return filePath;
}
