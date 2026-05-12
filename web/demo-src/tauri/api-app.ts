// Mock 替换 @tauri-apps/api/app
//
// 只有 AboutTab 用 getVersion() 显示版本号。Demo 里返回当前 main 仓库的 package.json 版本。

export async function getVersion(): Promise<string> {
  return "0.6.6";
}

export async function getName(): Promise<string> {
  return "Hindsight";
}

export async function getTauriVersion(): Promise<string> {
  return "2.0.0";
}
