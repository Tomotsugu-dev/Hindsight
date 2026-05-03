/**
 * 把进程名渲染成展示名：去掉常见可执行后缀 (.exe / .app / .lnk 等)，
 * 以及 Windows ".lnk" 链接、macOS bundle ".app"。原值仍用于 key / 后端 id。
 */
export function displayAppName(name: string): string {
  if (!name) return name;
  return name.replace(/\.(exe|lnk|app)$/i, "");
}
