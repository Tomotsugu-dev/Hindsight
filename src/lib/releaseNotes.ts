/** 更新通知里的 release note 按界面语言挑一段显示。
 *
 * release.yml 把整个 `docs/release-notes/<tag>.md` 当 GitHub Release body + updater
 * 的 `latest.json` notes 字段，前端 `update.body` 拿到的是这一整坨（含所有语言）。
 * tauri updater 的 notes 是单字符串、不支持按语言分发，所以在 `.md` 里给每个语言块
 * 加一行隐藏标记 `<!-- pt -->`（GitHub 渲染页不可见），前端按界面语言抽出对应块。
 *
 * 兼容：
 *  - 老版本 release 的 body 没有标记 → 原样返回整坨（不丢信息）。
 *  - 当前语言没有对应块 → 回退英文块 → 再回退第一个语言块（去标记）→ 最后整坨。
 */

/** 从带 `<!-- xx -->` 语言标记的 body 里挑出 `locale` 对应的那段；无标记 / 无匹配按上述兜底。 */
export function pickReleaseNotesForLang(body: string, locale: string): string {
  const re = /<!--\s*([a-z]{2})\s*-->/gi;
  const marks: { code: string; markStart: number; contentStart: number }[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(body)) !== null) {
    marks.push({
      code: m[1].toLowerCase(),
      markStart: m.index,
      contentStart: m.index + m[0].length,
    });
  }
  if (marks.length === 0) return body.trim();

  const sections: Record<string, string> = {};
  for (let i = 0; i < marks.length; i++) {
    const end = i + 1 < marks.length ? marks[i + 1].markStart : body.length;
    sections[marks[i].code] = body.slice(marks[i].contentStart, end).trim();
  }

  // "pt-BR" → "pt"、"zh-CN" → "zh"
  const want = locale.slice(0, 2).toLowerCase();
  return sections[want] ?? sections.en ?? Object.values(sections)[0] ?? body.trim();
}
