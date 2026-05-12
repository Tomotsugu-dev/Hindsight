// Mock 替换 @tauri-apps/plugin-process
//
// 主应用调 relaunch() 重启应用（如更换 data_root 后）。
// Demo 用 location.reload() 兜底。

export async function relaunch(): Promise<void> {
  if (typeof window !== "undefined") {
    window.location.reload();
  }
}

export async function exit(_code?: number): Promise<void> {
  // Demo 里"退出应用"没意义；至少不要崩
  // eslint-disable-next-line no-console
  console.warn("[demo] exit() 在 demo 模式下不可用");
}
