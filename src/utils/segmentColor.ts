// 段 chip 颜色解析的单一来源。
//
// 历史 bug：AISettings 的 SegmentList 里"未配置颜色"会回退到按时段中点
// 自动 HSL 渐变（早亮晚暗），而 AISummary 的 DailyTab / DebugTab 各自
// 写了 fallback = 固定 #cbd5e1 浅灰。同一个 seg 在两个页面颜色不同。
// 抽成共用工具，三处都调它，保证设置和总结视觉一致。

type HSL = { h: number; s: number; l: number };

const COLOR_STOPS: Array<{ hour: number; hsl: HSL }> = [
  { hour: 0, hsl: { h: 228, s: 50, l: 70 } },
  { hour: 5, hsl: { h: 258, s: 55, l: 76 } },
  { hour: 7.5, hsl: { h: 28, s: 78, l: 86 } },
  { hour: 10.5, hsl: { h: 45, s: 75, l: 82 } },
  { hour: 14, hsl: { h: 22, s: 65, l: 80 } },
  { hour: 17, hsl: { h: 345, s: 62, l: 82 } },
  { hour: 20, hsl: { h: 275, s: 45, l: 76 } },
  { hour: 23, hsl: { h: 228, s: 50, l: 68 } },
  { hour: 24, hsl: { h: 228, s: 50, l: 70 } },
];

function lerpHue(a: number, b: number, t: number): number {
  const diff = (((b - a + 540) % 360) - 180);
  return (a + diff * t + 360) % 360;
}

function hourColor(h: number): HSL {
  const clamped = Math.max(0, Math.min(24, h));
  for (let i = 0; i < COLOR_STOPS.length - 1; i++) {
    const a = COLOR_STOPS[i];
    const b = COLOR_STOPS[i + 1];
    if (clamped >= a.hour && clamped <= b.hour) {
      const t = (clamped - a.hour) / (b.hour - a.hour);
      return {
        h: lerpHue(a.hsl.h, b.hsl.h, t),
        s: a.hsl.s + (b.hsl.s - a.hsl.s) * t,
        l: a.hsl.l + (b.hsl.l - a.hsl.l) * t,
      };
    }
  }
  return COLOR_STOPS[0].hsl;
}

function hslStr({ h, s, l }: HSL): string {
  return `hsl(${h.toFixed(0)}, ${s.toFixed(0)}%, ${l.toFixed(0)}%)`;
}

/** L > 60 的 HSL 视作浅色（chip 文字应用深色） */
function isLightHsl(hsl: HSL): boolean {
  return hsl.l > 60;
}

/** 用 perceived luminance 判 hex 颜色是否浅色 */
export function isLightHex(hex: string): boolean {
  const m = hex.match(/^#([0-9a-f]{6})$/i);
  if (!m) return true;
  const r = parseInt(m[1].slice(0, 2), 16);
  const g = parseInt(m[1].slice(2, 4), 16);
  const b = parseInt(m[1].slice(4, 6), 16);
  const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return lum > 0.6;
}

interface SegmentColorInput {
  startHour: number;
  endHour: number;
  color: string;
}

/**
 * 统一的段 chip 背景色解析。
 * - 用户配过具体 hex → 用那个，文字明暗按 luminance 判
 * - 没配 → 按段中点的色温自动渐变（早亮晚暗），文字明暗按 HSL.l 判
 *
 * 返回的 background 可以直接塞 `style={{ background }}`，
 * isLight 用来选 chip 文字颜色（true → 深色字 #3a3f55，false → 白字）。
 */
export function resolveSegmentChip(seg: SegmentColorInput): {
  background: string;
  isLight: boolean;
} {
  const hasCustom = seg.color && seg.color.trim().length > 0;
  if (hasCustom) {
    return { background: seg.color, isLight: isLightHex(seg.color) };
  }
  const mid = (seg.startHour + seg.endHour) / 2;
  const hsl = hourColor(mid);
  return { background: hslStr(hsl), isLight: isLightHsl(hsl) };
}
