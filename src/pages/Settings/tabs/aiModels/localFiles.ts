import type { ModelEntry, RecommendedModel } from "../../../../api/hindsight";

/** A model on disk, as "My models" lists it. */
export type LocalItem =
  /** A recommended model that is downloaded. */
  | { kind: "recommended"; rec: RecommendedModel; main: ModelEntry; mmproj: ModelEntry | null }
  /** A file the user imported or put in the models folder. */
  | { kind: "file"; main: ModelEntry; mmproj: ModelEntry | null }
  /** A vision projection file with no model to go with it. It cannot be used alone. */
  | { kind: "orphanMmproj"; file: ModelEntry };

/**
 * Return the file name a recommended model's mmproj is saved under: `<main stem>__<mmproj name>`.
 *
 * Many repos call their mmproj `mmproj-F16.gguf`, so the saved name carries the main file to keep
 * two models from overwriting each other. Returns "" when the model has no mmproj.
 */
export function mmprojSaveAs(rec: RecommendedModel): string {
  if (!rec.mmprojFile) return "";
  const stem = rec.mainFile.replace(/\.gguf$/i, "");
  return `${stem}__${rec.mmprojFile}`;
}

/** Return true when the model's main file is on disk. The mmproj is not needed to use it. */
export function isRecommendedInstalled(rec: RecommendedModel, local: ModelEntry[]): boolean {
  return local.some((m) => m.filename === rec.mainFile);
}

// 文件名切成段：去 .gguf 后缀、去开头的 mmproj 前缀、小写。
// gemma-4-12B-it-Q4_K_M → [gemma,4,12b,it,q4,k,m]
function fileSegments(name: string): string[] {
  return name
    .replace(/\.gguf$/i, "")
    .replace(/^mmproj[-_]?/i, "")
    .toLowerCase()
    .split(/[-_.]/)
    .filter(Boolean);
}

function commonPrefixLen(a: string[], b: string[]): number {
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  return i;
}

// The mmproj whose name shares the longest prefix with the main file. At least two segments
// (like gemma-4) must match, so a 12B mmproj does not go with a 31B model.
function autoPairMmproj(mainName: string, mmprojs: ModelEntry[]): ModelEntry | null {
  const ms = fileSegments(mainName);
  let best: ModelEntry | null = null;
  let bestLen = 1;
  for (const p of mmprojs) {
    const len = commonPrefixLen(ms, fileSegments(p.filename));
    if (len > bestLen) {
      bestLen = len;
      best = p;
    }
  }
  return best;
}

/**
 * Group the files on disk into models: downloaded recommended models first, in the list's order,
 * then other files, then mmproj files that no model goes with.
 *
 * A file that is not in the recommended list gets the mmproj whose name matches it best. When
 * names do not match and exactly one file and one mmproj are left, they go together.
 */
export function groupLocalFiles(local: ModelEntry[], recommended: RecommendedModel[]): LocalItem[] {
  const byName = new Map(local.map((m) => [m.filename, m]));
  const items: LocalItem[] = [];
  const recNames = new Set<string>();

  for (const rec of recommended) {
    recNames.add(rec.mainFile);
    const mmprojName = mmprojSaveAs(rec);
    if (mmprojName) recNames.add(mmprojName);
    const main = byName.get(rec.mainFile);
    if (!main) continue;
    items.push({
      kind: "recommended",
      rec,
      main,
      mmproj: (mmprojName && byName.get(mmprojName)) || null,
    });
  }

  const others = local.filter((m) => !recNames.has(m.filename));
  const mmprojs = others.filter((m) => m.isMmproj);
  const paired = new Set<string>();
  // 第一轮按文件名前缀配；每次只从还没配走的 mmproj 里挑，免得两个主模型抢同一个
  const files = others
    .filter((m) => !m.isMmproj)
    .map((main) => {
      const mmproj = autoPairMmproj(
        main.filename,
        mmprojs.filter((m) => !paired.has(m.filename)),
      );
      if (mmproj) paired.add(mmproj.filename);
      return { kind: "file" as const, main, mmproj };
    });
  // 第二轮兜底：LM Studio 这类给 mmproj 起通用名（mmproj-model-f16.gguf），前缀配不上；
  // 只剩一个主模型和一个 mmproj 时没有歧义，直接配
  const unpairedFiles = files.filter((f) => !f.mmproj);
  const leftMmprojs = mmprojs.filter((m) => !paired.has(m.filename));
  if (unpairedFiles.length === 1 && leftMmprojs.length === 1) {
    unpairedFiles[0].mmproj = leftMmprojs[0];
    paired.add(leftMmprojs[0].filename);
  }
  items.push(...files);

  for (const file of mmprojs) {
    if (!paired.has(file.filename)) items.push({ kind: "orphanMmproj", file });
  }
  return items;
}
