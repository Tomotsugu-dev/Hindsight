/**
 * 把 (key, value) 写入 Map，size 超过 max 时按"插入顺序"从头部 evict 掉最早条目。
 *
 * 注意：已存在的 key 重新 set 时**不会**移到末尾——保持插入顺序语义，跟
 * useHourApps.ts 里的内联 LRU 行为一致；既有 prefetch ±1 的访问模式下，
 * 邻居在插入序里已经接近末尾，纯 FIFO 已经够用，没必要再 delete-then-set
 * 把已 cached 条目反复"刷新"到末尾（前者会让 Map 内部链表少一次修改）。
 *
 * 原地 mutate 入参 Map。调用方负责好不变性（比如在 React setState 里先
 * `new Map(prev)` 再 mutate）。
 */
export function lruInsert<K, V>(
  map: Map<K, V>,
  key: K,
  value: V,
  max: number,
): void {
  map.set(key, value);
  while (map.size > max) {
    const oldest = map.keys().next().value;
    if (oldest === undefined) break;
    map.delete(oldest);
  }
}
