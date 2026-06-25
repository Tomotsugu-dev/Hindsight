// AI 时段默认标签的语言跟随。
//
// 后端首次启动时按系统 locale 把默认时段 seed 进设置（[crate::ai::config::default_segments_for]）。
// 那是一次性的存储数据，之后切 UI 语言不会自动重译。这里补上：切语言时，若时段**仍是某语言
// 的默认**（区间全等默认 + 标签整组命中某默认集），就把标签重译成新语言的默认；用户一旦改过
// 标签 / 区间 / 增删段，就判定为非默认，绝不动。颜色始终保留。

import type { AiSegment } from "../api/hindsight";

/** 固定的 5 段区间（0-6 / 6-9 / 9-12 / 12-18 / 18-24）。必须与后端 default_segments_for 一致。 */
const DEFAULT_RANGES: ReadonlyArray<readonly [number, number]> = [
  [0, 6],
  [6, 9],
  [9, 12],
  [12, 18],
  [18, 24],
];

/** 各语言默认时段标签（顺序对应 DEFAULT_RANGES）。**镜像**后端 config.rs::default_segments_for，
 *  两边改一处就得同步另一处，否则"是否默认"会判失准。 */
const DEFAULT_SEGMENT_LABELS: Record<string, readonly string[]> = {
  "zh-CN": ["深夜", "早上", "上午", "下午", "晚上"],
  en: ["Late Night", "Early Morning", "Morning", "Afternoon", "Evening"],
  ja: ["深夜", "早朝", "午前", "午後", "夜"],
  "pt-BR": ["Madrugada", "Manhã cedo", "Manhã", "Tarde", "Noite"],
};

/** i18n locale → 默认标签表 key（zh*→zh-CN, ja*→ja, pt*→pt-BR, 其余→en）。 */
function localeKey(lang: string): string {
  const l = lang.toLowerCase();
  if (l.startsWith("zh")) return "zh-CN";
  if (l.startsWith("ja")) return "ja";
  if (l.startsWith("pt")) return "pt-BR";
  return "en";
}

/** 当前时段是否仍是"某语言的默认"：区间全等默认 + 标签整组命中某语言默认集。
 *  颜色不参与判断（只改过颜色仍算默认标签）。 */
export function isDefaultSegments(segments: AiSegment[]): boolean {
  if (segments.length !== DEFAULT_RANGES.length) return false;
  const rangesMatch = segments.every(
    (s, i) =>
      s.startHour === DEFAULT_RANGES[i][0] && s.endHour === DEFAULT_RANGES[i][1],
  );
  if (!rangesMatch) return false;
  return Object.values(DEFAULT_SEGMENT_LABELS).some((set) =>
    set.every((label, i) => segments[i].label === label),
  );
}

/** 把默认时段的标签重译成目标语言（保留区间与颜色，只换 label）。
 *  仅当 [isDefaultSegments] 为真时调用。 */
export function retranslateDefaultSegments(
  segments: AiSegment[],
  lang: string,
): AiSegment[] {
  const labels = DEFAULT_SEGMENT_LABELS[localeKey(lang)] ?? DEFAULT_SEGMENT_LABELS.en;
  return segments.map((s, i) => ({ ...s, label: labels[i] ?? s.label }));
}
