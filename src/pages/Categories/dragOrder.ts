/** 拖拽命中一行：插到这一行前面还是后面。 */
export interface RowDrop {
  rowId: string;
  after: boolean;
}

/** 拖拽开始时快照的一行；坐标是滚动内容坐标。 */
export interface RowBand {
  id: string;
  superKey: string;
  top: number;
  bottom: number;
}

/** 两个大类之间的空隙；坐标同样是滚动内容坐标。 */
export interface GapBand {
  afterSuperId: string;
  top: number;
  bottom: number;
  lineTop: number;
}

/** 小类拖拽的最终落点。rowId = null 表示落到空大类。 */
export type CategoryDropIntent =
  | {
      kind: "row";
      superKey: string;
      rowId: string | null;
      after: boolean;
    }
  | {
      kind: "promote";
      afterSuperId: string;
    };

/** 大类拖拽的最终落点。 */
export type SuperDropIntent =
  | {
      kind: "reorder";
      targetSuperId: string;
    }
  | {
      kind: "demote";
      targetSuperId: string;
      targetRowId: string | null;
      after: boolean;
    };

/** 空隙中线上下各 8px 命中升级。 */
export const PROMOTE_GAP_HALF_HEIGHT = 8;

/**
 * 在给定行里找光标对应的插入位置。
 *
 * 行内按上半区/下半区决定前/后；落在行间空隙时选最近的一行；
 * 高于第一行落到第一行前，低于最后一行落到最后一行后。
 */
export function pickRowDrop(rows: readonly RowBand[], y: number): RowDrop | null {
  if (rows.length === 0) return null;

  let best: RowBand | null = null;
  let bestDistance = Infinity;
  for (const row of rows) {
    const distance = y < row.top ? row.top - y : y > row.bottom ? y - row.bottom : 0;
    if (distance < bestDistance) {
      bestDistance = distance;
      best = row;
    }
  }
  if (!best) return null;

  if (y <= best.top) return { rowId: best.id, after: false };
  if (y >= best.bottom) return { rowId: best.id, after: true };
  return { rowId: best.id, after: y >= (best.top + best.bottom) / 2 };
}

/** 光标是否命中了组间空隙；空隙优先于组内行命中。 */
export function pickPromoteGap(
  gaps: readonly GapBand[],
  y: number,
  tolerance = PROMOTE_GAP_HALF_HEIGHT,
): string | null {
  for (const gap of gaps) {
    if (Math.abs(y - gap.lineTop) <= tolerance) return gap.afterSuperId;
  }
  return null;
}

export function sameCategoryDrop(
  a: CategoryDropIntent | null,
  b: CategoryDropIntent | null,
): boolean {
  if (a === b) return true;
  if (!a || !b || a.kind !== b.kind) return false;
  if (a.kind === "promote" && b.kind === "promote") {
    return a.afterSuperId === b.afterSuperId;
  }
  if (a.kind === "row" && b.kind === "row") {
    return a.superKey === b.superKey && a.rowId === b.rowId && a.after === b.after;
  }
  return false;
}

export function sameSuperDrop(a: SuperDropIntent | null, b: SuperDropIntent | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.kind !== b.kind) return false;
  if (a.kind === "reorder" && b.kind === "reorder") {
    return a.targetSuperId === b.targetSuperId;
  }
  if (a.kind === "demote" && b.kind === "demote") {
    return (
      a.targetSuperId === b.targetSuperId && a.targetRowId === b.targetRowId && a.after === b.after
    );
  }
  return false;
}

/** 把单个条目移到目标条目前/后，保持其它条目顺序。 */
export function moveBeforeOrAfter(
  ids: readonly string[],
  movingId: string,
  targetId: string,
  after: boolean,
): string[] {
  if (movingId === targetId) return [...ids];
  const without = ids.filter((id) => id !== movingId);
  const targetIndex = without.indexOf(targetId);
  if (targetIndex < 0) return [...ids];
  const insertAt = targetIndex + (after ? 1 : 0);
  return [...without.slice(0, insertAt), movingId, ...without.slice(insertAt)];
}

/** 把一整块条目（降级大类的子分类）按原相对顺序移到目标条目前/后。 */
export function moveBlockBeforeOrAfter(
  ids: readonly string[],
  movingIds: readonly string[],
  targetId: string,
  after: boolean,
): string[] {
  const moving = new Set(movingIds);
  const block = ids.filter((id) => moving.has(id));
  if (block.length === 0) return [...ids];

  const without = ids.filter((id) => !moving.has(id));
  const targetIndex = without.indexOf(targetId);
  if (targetIndex < 0) return [...ids];
  const insertAt = targetIndex + (after ? 1 : 0);
  return [...without.slice(0, insertAt), ...block, ...without.slice(insertAt)];
}

/** 大类降级时计算子分类块的全局顺序；空目标按大类在页面上的位置插入。 */
export function planDemoteOrder(
  ids: readonly string[],
  movingIds: readonly string[],
  targetSuperKey: string,
  targetRowId: string | null,
  after: boolean,
  superOrder: readonly string[],
  superKeyOf: (id: string) => string,
): string[] {
  if (targetRowId) {
    return moveBlockBeforeOrAfter(ids, movingIds, targetRowId, after);
  }
  return insertBlockAtSuperPosition(ids, movingIds, targetSuperKey, superOrder, superKeyOf);
}

/** 升级新大类时，在指定大类后面插入它的 id。 */
export function insertSuperAfter(
  supers: readonly string[],
  afterSuperId: string,
  newSuperId: string,
): string[] | null {
  const index = supers.indexOf(afterSuperId);
  if (index < 0) return null;
  return [...supers.slice(0, index + 1), newSuperId, ...supers.slice(index + 1)];
}

/**
 * 目标大类还没有任何行时，把整块插到它在页面上的可视位置。
 *
 * `superOrder` 是「所有真大类 + 未归入」的显示顺序；找不到的 key
 * （builtin）视为排在最后，因此新块会落在 builtin 之前。
 */
export function insertBlockAtSuperPosition(
  ids: readonly string[],
  movingIds: readonly string[],
  targetSuperKey: string,
  superOrder: readonly string[],
  superKeyOf: (id: string) => string,
): string[] {
  const moving = new Set(movingIds);
  const block = ids.filter((id) => moving.has(id));
  if (block.length === 0) return [...ids];

  const without = ids.filter((id) => !moving.has(id));
  const targetIndex = superOrder.indexOf(targetSuperKey);
  let insertAt = without.length;
  if (targetIndex >= 0) {
    for (let i = 0; i < without.length; i += 1) {
      const index = superOrder.indexOf(superKeyOf(without[i]));
      if (index < 0 || index > targetIndex) {
        insertAt = i;
        break;
      }
    }
  }
  return [...without.slice(0, insertAt), ...block, ...without.slice(insertAt)];
}

/** 顺序完全一致时跳过无意义的后端写入。 */
export function sameOrder(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((id, index) => id === b[index]);
}
