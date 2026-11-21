import { describe, expect, it, vi } from "vitest";
import { hydratePoster, hydrateVideo, initHomeMedia, isDataSaver } from "./home-media.js";

function makeVideo({ src, poster }) {
  const attrs = new Map();
  const listeners = new Map();
  return {
    dataset: { src, poster },
    preload: "none",
    readyState: 0,
    play: vi.fn(() => Promise.resolve()),
    pause: vi.fn(),
    load: vi.fn(),
    setAttribute(name, value) {
      attrs.set(name, value);
    },
    getAttribute(name) {
      return attrs.get(name) ?? null;
    },
    addEventListener(type, fn) {
      const list = listeners.get(type) ?? [];
      list.push(fn);
      listeners.set(type, list);
    },
    removeEventListener(type, fn) {
      const list = listeners.get(type) ?? [];
      listeners.set(
        type,
        list.filter((f) => f !== fn),
      );
    },
    emit(type) {
      if (type === "loadeddata") this.readyState = 2;
      for (const fn of [...(listeners.get(type) ?? [])]) fn();
    },
  };
}

describe("home-media helpers", () => {
  it("detects Save-Data and 2G connections", () => {
    expect(isDataSaver(undefined)).toBe(false);
    expect(isDataSaver({ connection: { saveData: false, effectiveType: "4g" } })).toBe(false);
    expect(isDataSaver({ connection: { saveData: true } })).toBe(true);
    expect(isDataSaver({ connection: { effectiveType: "2g" } })).toBe(true);
    expect(isDataSaver({ connection: { effectiveType: "slow-2g" } })).toBe(true);
  });

  it("hydrates poster and video lazily without resetting src when upgrading preload", () => {
    const v = makeVideo({ src: "/assets/video/demo.mp4", poster: "/assets/video/demo.jpg" });

    expect(hydratePoster(v)).toBe(true);
    expect(v.getAttribute("poster")).toBe("/assets/video/demo.jpg");
    expect(hydratePoster(v)).toBe(false);

    expect(hydrateVideo(v, "metadata")).toBe(true);
    expect(v.getAttribute("src")).toBe("/assets/video/demo.mp4");
    expect(v.preload).toBe("metadata");
    expect(v.load).toHaveBeenCalledTimes(1);

    // 再次升级为 auto：只改 preload，不重复触发 load()
    expect(hydrateVideo(v, "auto")).toBe(false);
    expect(v.preload).toBe("auto");
    expect(v.load).toHaveBeenCalledTimes(1);
  });
});

describe("initHomeMedia progressive cascade", () => {
  it("starts with 0 video/poster loads, then hydrates cards in order after hero demo loads", async () => {
    vi.useFakeTimers();
    try {
      const cards = [0, 1, 2, 3, 4].map((i) =>
        makeVideo({ src: `/assets/home/c${i}.mp4`, poster: `/assets/home/c${i}.webp` }),
      );
      const film = makeVideo({ src: "/assets/video/Hindsight_zh.mp4", poster: "/assets/video/Hindsight_zh.jpg" });
      const demoListeners = new Map();
      const demoFrame = {
        addEventListener: (t, fn) => demoListeners.set(t, fn),
        removeEventListener: (t) => demoListeners.delete(t),
      };

      const observers = [];
      class FakeIO {
        constructor(cb, opts) {
          this.cb = cb;
          this.opts = opts;
          this.targets = new Set();
          observers.push(this);
        }
        observe(el) {
          this.targets.add(el);
        }
        unobserve(el) {
          this.targets.delete(el);
        }
        disconnect() {
          this.targets.clear();
        }
      }

      const winListeners = new Map();
      const fakeWin = {
        IntersectionObserver: FakeIO,
        matchMedia: () => ({ matches: false }),
        navigator: { connection: { saveData: false, effectiveType: "4g" } },
        scrollY: 0,
        setTimeout,
        clearTimeout,
        addEventListener: (t, fn) => winListeners.set(t, fn),
        removeEventListener: (t) => winListeners.delete(t),
      };
      const fakeDoc = {
        querySelectorAll: (sel) => (sel === ".card-media video" ? cards : []),
        querySelector: (sel) => (sel === ".film video" ? film : sel === ".hero-demo iframe" ? demoFrame : null),
        addEventListener: () => {},
        removeEventListener: () => {},
      };

      const dispose = initHomeMedia(fakeDoc, fakeWin);

      // P0：刚进入页面时下方视频和封面全部 0 请求
      for (const v of [...cards, film]) {
        expect(v.getAttribute("src")).toBeNull();
        expect(v.getAttribute("poster")).toBeNull();
      }

      // 顶部 demo iframe 加载完 -> 留 500ms 给首屏语言包/图片，随后触发 P1 接力
      demoListeners.get("load")();
      await vi.advanceTimersByTimeAsync(550);

      // 前 4 张卡片封面已挂，第 1 张卡片视频已开始预热
      expect(cards[0].getAttribute("poster")).toBe("/assets/home/c0.webp");
      expect(cards[3].getAttribute("poster")).toBe("/assets/home/c3.webp");
      expect(cards[0].getAttribute("src")).toBe("/assets/home/c0.mp4");
      // 第 2 张仍在排队等待第 1 张首帧或超时，不并发轰炸
      expect(cards[1].getAttribute("src")).toBeNull();

      // 第 1、2、3 张依次加载完成
      cards[0].emit("loadeddata");
      await Promise.resolve();
      await Promise.resolve();
      expect(cards[1].getAttribute("src")).toBe("/assets/home/c1.mp4");
      cards[1].emit("loadeddata");
      await Promise.resolve();
      await Promise.resolve();
      expect(cards[2].getAttribute("src")).toBe("/assets/home/c2.mp4");
      cards[2].emit("loadeddata");
      await Promise.resolve();
      await Promise.resolve();

      // 没往下滑时：底部大视频只挂封面，绝不提前拉 11 MB 视频流
      expect(film.getAttribute("poster")).toBe("/assets/video/Hindsight_zh.jpg");
      expect(film.getAttribute("src")).toBeNull();

      // 用户开始往下滑 -> 立即预拉大视频 metadata
      fakeWin.scrollY = 200;
      winListeners.get("scroll")();
      expect(film.getAttribute("src")).toBe("/assets/video/Hindsight_zh.mp4");
      expect(film.preload).toBe("metadata");

      // 滚进大视频视口 -> 升级 auto 并自动播放；滚出即暂停
      const filmPlayObserver = observers.find((o) => o.opts?.threshold === 0.25);
      filmPlayObserver.cb([{ isIntersecting: true, target: film }]);
      expect(film.preload).toBe("auto");
      expect(film.play).toHaveBeenCalledTimes(1);

      filmPlayObserver.cb([{ isIntersecting: false, target: film }]);
      expect(film.pause).toHaveBeenCalledTimes(1);

      dispose();
    } finally {
      vi.useRealTimers();
    }
  });
});
