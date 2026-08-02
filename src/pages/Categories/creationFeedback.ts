/**
 * 新建分类 / 大类之后的"滚到它"决策。
 *
 * 抽成纯函数是为了锁住一条容易写错、写错了又很难在 UI 上察觉的规则:
 * **同一个新建只滚一次**。SuperCategoriesTable 每次 drag / hover / refresh 都会重渲染,
 * 若不记住"已经为这个 id 滚过了",每次重渲染都会把视口拽回新建那一行,
 * 用户拖着分类到别处时会莫名其妙被弹走。
 */

export type CreationKind = "cat" | "super";

export interface CreationScrollTarget {
  key: string;
  kind: CreationKind;
}

/**
 * 该滚到谁。返回 `null` = 这次渲染不该滚。
 *
 * - 分类优先于大类:两个 id 同时有值只会发生在"上一次新建的 id 还留着"的情况下,
 *   而调用方每次新建都会把另一个清空,所以取分类即取最新那次;
 * - `alreadyScrolledFor` 命中则不再滚(同一个新建只滚一次)。
 */
export function creationScrollTarget(
  justCreatedCatId: string | null,
  justCreatedSuperId: string | null,
  alreadyScrolledFor: string | null,
): CreationScrollTarget | null {
  const target: CreationScrollTarget | null = justCreatedCatId
    ? { key: justCreatedCatId, kind: "cat" }
    : justCreatedSuperId
      ? { key: justCreatedSuperId, kind: "super" }
      : null;
  if (!target) return null;
  return target.key === alreadyScrolledFor ? null : target;
}
