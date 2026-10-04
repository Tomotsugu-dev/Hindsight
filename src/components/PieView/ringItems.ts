/** 同心环里的一圈：一个小类 */
export interface RingItem {
  id: string;
  name: string;
  color: string;
  minutes: number;
  /** 所属大类 */
  superId: string;
}

export const OTHERS_ID = "__others__";

/** 鼠标停在下方应用列表某一行时，同心环要展开的那个应用 */
export interface RingFocusApp {
  groupId: string;
  name: string;
  /** 组里一个真实的 process_name，用来查图标 */
  iconProcess: string;
  categoryId: string;
  /** 所属小类的颜色 */
  color: string;
  minutes: number;
}

/** 一个完整圆环上的一段：一个小类，或者合起来的「其他」 */
export interface RingSegment {
  id: string;
  name: string;
  color: string;
  minutes: number;
  /** 「其他」那一段合了几个小类；普通的一段为 0 */
  merged: number;
}

/**
 * 把一整圈分给各段，太小画不出来的段并进「其他」。
 * - `minFraction`：一段至少要占整圈的这么多，才放得下两头的圆头和段间的缝。
 * - 只有一段不够时保留它自己，两段及以上才合成「其他」。
 * - 合完还不够 minFraction 的那一段，画的时候按 minFraction 画，多出来的从最大那段扣；
 *   `fractions` 是画的比例，真实比例按 minutes 算。
 */
export function fitSegments(
  items: RingSegment[],
  minFraction: number,
  othersColor: string,
): { segments: RingSegment[]; fractions: number[] } {
  const sorted = items.filter((i) => i.minutes > 0).sort((a, b) => b.minutes - a.minutes);
  const total = sorted.reduce((sum, i) => sum + i.minutes, 0);
  if (total <= 0) return { segments: [], fractions: [] };

  const big = sorted.filter((i) => i.minutes / total >= minFraction);
  const small = sorted.filter((i) => i.minutes / total < minFraction);
  const segments =
    small.length >= 2
      ? [
          ...big,
          {
            id: OTHERS_ID,
            name: "",
            color: othersColor,
            minutes: small.reduce((sum, i) => sum + i.minutes, 0),
            merged: small.length,
          },
        ]
      : sorted;

  const fractions = segments.map((s) => s.minutes / total);
  if (segments.length > 1) {
    let lifted = 0;
    fractions.forEach((f, i) => {
      if (f < minFraction) {
        lifted += minFraction - f;
        fractions[i] = minFraction;
      }
    });
    // 最大那段排在最前面
    fractions[0] -= lifted;
  }
  return { segments, fractions };
}

/**
 * 同心环只露出 `rings` 圈，像一扇窗压在按分钟数从多到少排好的全部小类上。
 * `offset` 是窗口从第几名开始（0 起）：往下滑一格，最外圈那名退出、最内圈补进下一名。
 * offset 超出范围时夹到能放满的最后一格；返回夹过的 offset 和一共几名。
 */
export function ringWindow(
  items: RingItem[],
  rings: number,
  offset: number,
): { visible: RingItem[]; offset: number; total: number } {
  const sorted = items.filter((i) => i.minutes > 0).sort((a, b) => b.minutes - a.minutes);
  const maxOffset = Math.max(0, sorted.length - rings);
  const at = Math.min(Math.max(0, offset), maxOffset);
  return { visible: sorted.slice(at, at + rings), offset: at, total: sorted.length };
}

/**
 * 让 `id` 那一名露在窗口里，窗口挪得最少：在窗口上面就挪到它在最外圈，在下面就挪到它在最内圈。
 * 已经露着、或者找不到它时 offset 不变。
 */
export function offsetToShow(items: RingItem[], rings: number, offset: number, id: string): number {
  const sorted = items.filter((i) => i.minutes > 0).sort((a, b) => b.minutes - a.minutes);
  const at = sorted.findIndex((i) => i.id === id);
  if (at < 0) return offset;
  if (at < offset) return at;
  if (at >= offset + rings) return at - rings + 1;
  return offset;
}
