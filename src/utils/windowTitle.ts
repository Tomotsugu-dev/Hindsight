/** 去掉标题结尾冗余的 " - {app名}"（VS Code 等把 app 名拼在标题最后，纯重复）。 */
export function stripAppSuffix(title: string, appName: string): string {
  const t = title.trim();
  if (!appName) return t;
  for (const sep of [" - ", " — ", " – "]) {
    const suffix = sep + appName;
    if (t.endsWith(suffix)) return t.slice(0, -suffix.length).trim();
  }
  return t;
}
