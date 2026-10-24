// 测试用：theme.ts 用到的 localStorage、<html> 的 dataset 和 style、adoptedStyleSheets。
// vitest 跑在 node 环境（纯函数策略），不为这几样挂 jsdom。只给测试 import。
import { vi } from "vitest";

/** 假的 CSSStyleSheet：只记下 replaceSync 收到的文字。 */
export class FakeSheet {
  text = "";
  replaceSync(css: string): void {
    this.text = css;
  }
}

/** 装上假的全局对象，返回测试要看的几样。 */
export function installFakeThemeDom() {
  const store = new Map<string, string>();
  /** <html> 的 style 上设了的变量 */
  const tokens = new Map<string, string>();
  const doc = {
    documentElement: {
      dataset: {} as Record<string, string>,
      style: {
        setProperty: (name: string, value: string) => void tokens.set(name, value),
        removeProperty: (name: string) => {
          tokens.delete(name);
          return "";
        },
      },
    },
    adoptedStyleSheets: [] as FakeSheet[],
  };
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  });
  vi.stubGlobal("document", doc);
  vi.stubGlobal("CSSStyleSheet", FakeSheet);
  return {
    store,
    tokens,
    dataset: doc.documentElement.dataset,
    /** 现在加在页面上的 theme.css */
    sheets: () => doc.adoptedStyleSheets.map((s) => s.text),
  };
}
