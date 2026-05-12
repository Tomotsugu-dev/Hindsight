// Mock 替换 @tauri-apps/plugin-dialog
//
// Demo 里不能真打开系统保存对话框；用浏览器的下载机制兜底（用户主动点导出按钮时调用）。

export interface SaveDialogOptions {
  defaultPath?: string;
  filters?: Array<{ name: string; extensions: string[] }>;
  title?: string;
}

export interface OpenDialogOptions {
  multiple?: boolean;
  directory?: boolean;
  defaultPath?: string;
  filters?: Array<{ name: string; extensions: string[] }>;
  title?: string;
}

/**
 * Demo 模式：直接返回 default path（如果有），表示"用户选了这个位置"。
 * 调用方通常会接着把内容写到这个 path——demo 里写入会失败但不崩溃，对应组件应有 fallback。
 */
export async function save(options?: SaveDialogOptions): Promise<string | null> {
  // eslint-disable-next-line no-console
  console.warn("[demo] save() 在 demo 模式下返回 null，不会真打开保存对话框");
  return options?.defaultPath ?? null;
}

export async function open(_options?: OpenDialogOptions): Promise<string | string[] | null> {
  // eslint-disable-next-line no-console
  console.warn("[demo] open() 在 demo 模式下返回 null");
  return null;
}

export async function message(_msg: string, _options?: unknown): Promise<void> {
  // demo 里用 alert 兜底
  // eslint-disable-next-line no-console
  console.warn("[demo] message() 被调用，不会弹原生对话框");
}

export async function confirm(_msg: string, _options?: unknown): Promise<boolean> {
  // demo 里默认确认
  return true;
}

export async function ask(_msg: string, _options?: unknown): Promise<boolean> {
  return true;
}
