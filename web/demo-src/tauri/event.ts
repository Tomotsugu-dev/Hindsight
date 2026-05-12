// Mock 替换 @tauri-apps/api/event
//
// 主应用用 listen() 订阅后端事件（下载进度、AI 总结进度等）。
// Demo 里事件由 api-mock 内部用 setTimeout 模拟，通过本模块的 emit() 分发。

export type UnlistenFn = () => void;

export interface Event<T> {
  event: string;
  payload: T;
  id: number;
  windowLabel?: string;
}

type Handler<T> = (event: Event<T>) => void;

const handlers = new Map<string, Set<Handler<unknown>>>();
let nextId = 1;

export async function listen<T>(
  event: string,
  handler: Handler<T>,
): Promise<UnlistenFn> {
  if (!handlers.has(event)) handlers.set(event, new Set());
  const set = handlers.get(event)!;
  set.add(handler as Handler<unknown>);
  return () => {
    set.delete(handler as Handler<unknown>);
  };
}

export async function emit<T>(event: string, payload?: T): Promise<void> {
  const set = handlers.get(event);
  if (!set) return;
  const ev: Event<T> = {
    event,
    payload: payload as T,
    id: nextId++,
  };
  for (const handler of Array.from(set)) {
    try {
      handler(ev as Event<unknown>);
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error(`[demo] listener error for "${event}":`, err);
    }
  }
}

// once 在主应用某些地方有用到，简单实现：listen 一次后自动 unlisten
export async function once<T>(
  event: string,
  handler: Handler<T>,
): Promise<UnlistenFn> {
  const unlisten = await listen<T>(event, (ev) => {
    unlisten();
    handler(ev);
  });
  return unlisten;
}
