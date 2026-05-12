// Mock 替换 @tauri-apps/plugin-updater
//
// 主应用用 check() 查更新；demo 里不需要更新机制，直接返回"无更新"。

export interface Update {
  available: boolean;
  currentVersion: string;
  version: string;
  date?: string;
  body?: string;
  downloadAndInstall(onEvent?: (event: unknown) => void): Promise<void>;
}

export async function check(): Promise<Update | null> {
  // Demo 里始终返回 null（"已是最新版"），UpdaterProvider 不会显示更新提示
  return null;
}
