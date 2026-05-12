// Mock 替换 @tauri-apps/plugin-os
//
// Demo 里固定返回 "windows"——主应用看到这个会按 Windows 风格渲染 chrome（如标题栏、滚动条样式）。

export type Platform = "linux" | "macos" | "ios" | "freebsd" | "dragonfly" | "netbsd" | "openbsd" | "solaris" | "android" | "windows";

export function type(): Platform {
  return "windows";
}

export function platform(): Platform {
  return "windows";
}

export function arch(): string {
  return "x86_64";
}

export function version(): string {
  return "demo";
}

export function family(): "unix" | "windows" {
  return "windows";
}

export function locale(): Promise<string | null> {
  return Promise.resolve("zh-CN");
}
