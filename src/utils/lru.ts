/**
 * 把 (key, value) 写入 Map，size 超过 max 时从头部 evict 掉最久未写入的条目。
 *
 * 已存在的 key 重新 set 会先 delete 再 set（移到末尾续活）——纯 FIFO 会把
 * "每次翻页都在重写的 ±1 预取邻居"当老条目误杀，滑动切页时侧边 slide 闪空白
 * 再多发一次请求；delete-then-set 让活跃条目始终在队尾，被淘汰的才是真没人碰的。
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
  map.delete(key);
  map.set(key, value);
  while (map.size > max) {
    const oldest = map.keys().next().value;
    if (oldest === undefined) break;
    map.delete(oldest);
  }
}
