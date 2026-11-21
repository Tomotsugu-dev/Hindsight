// 官网首页视频与封面的分级动态加载：
// - P0：首屏基础页面 + 顶部交互 demo iframe 绝对优先，下方视频与封面初始 0 请求。
// - P1：顶部 demo 加载完（或有明确向下滑动/点锚点意向）后，按从上到下顺序接力预热功能卡片封面与首批可视卡片视频。
// - P2：继续空闲时依次补齐其余卡片封面与底部大视频封面；有下滑意向后再加载底部视频元数据，滚近时缓冲首段。
// - P3：进入视口才播放，滚出即暂停；任意时刻滚动/跳锚点都由 IntersectionObserver 立即插队加载当前可见项。

/** 判断是否处于省流量模式或极慢蜂窝网络：此时不做后台预热，只保留滚到视口再加载。 */
export function isDataSaver(nav = typeof navigator !== "undefined" ? navigator : undefined) {
  const conn = nav && /** @type {{ connection?: { saveData?: boolean; effectiveType?: string } }} */ (nav).connection;
  if (!conn) return false;
  return Boolean(conn.saveData || conn.effectiveType === "2g" || conn.effectiveType === "slow-2g");
}

/** 挂载封面图（幂等）。 */
export function hydratePoster(video) {
  const poster = video.dataset?.poster;
  if (!poster) return false;
  video.setAttribute("poster", poster);
  delete video.dataset.poster;
  return true;
}

/**
 * 挂载视频真实地址（幂等）。
 * preloadMode 为 "metadata" 时只拉元数据和首帧头，避免提前吞掉 11 MB 大视频流量；
 * 若已挂载且需要升到 "auto"，只升级 preload 属性不重设 src。
 */
export function hydrateVideo(video, preloadMode = "metadata") {
  hydratePoster(video);
  const src = video.dataset?.src;
  if (src) {
    video.preload = preloadMode;
    video.setAttribute("src", src);
    delete video.dataset.src;
    if (typeof video.load === "function") video.load();
    return true;
  }
  if (preloadMode === "auto" && video.preload !== "auto") {
    video.preload = "auto";
  }
  return false;
}

/** 等待单个视频至少拿到首帧（或超时兜底），用于串行接力，避免并发占满连接池。 */
function waitLoadedOrTimeout(video, timeoutMs) {
  if (video.readyState >= 2) return Promise.resolve();
  return new Promise((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      video.removeEventListener("loadeddata", finish);
      video.removeEventListener("error", finish);
      clearTimeout(timer);
      resolve();
    };
    const timer = setTimeout(finish, timeoutMs);
    video.addEventListener("loadeddata", finish, { once: true });
    video.addEventListener("error", finish, { once: true });
  });
}

function nextFrame() {
  if (typeof requestAnimationFrame === "function") {
    return new Promise((resolve) => requestAnimationFrame(() => resolve()));
  }
  return Promise.resolve();
}

export function initHomeMedia(doc = document, win = window) {
  const cardVideos = [...doc.querySelectorAll(".card-media video")];
  const filmVideo = doc.querySelector(".film video");
  const allVideos = filmVideo ? [...cardVideos, filmVideo] : cardVideos;
  if (allVideos.length === 0) return () => {};

  const reducedMotion =
    typeof win.matchMedia === "function" && win.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const saveData = isDataSaver(win.navigator);

  // 极老浏览器无 IntersectionObserver 时直接退回原生 src/poster
  if (typeof win.IntersectionObserver !== "function") {
    allVideos.forEach((v) => hydrateVideo(v, "metadata"));
    return () => {};
  }

  let disposed = false;
  let cascadeStarted = false;
  const cleanups = [];

  // 1. 插队预加载 Observer：无论后台队列跑没跑到，滚到附近立刻加载
  const cardNearObserver = new win.IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        hydrateVideo(e.target, reducedMotion ? "metadata" : "auto");
        cardNearObserver.unobserve(e.target);
      }
    },
    { rootMargin: "350px 240px" },
  );
  cardVideos.forEach((v) => cardNearObserver.observe(v));
  cleanups.push(() => cardNearObserver.disconnect());

  if (filmVideo) {
    const filmNearObserver = new win.IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          // 滚到功能区附近（距大视频 700px 内）：开始缓冲首段，保证进视口即秒播；减少动效时只拉 metadata
          hydrateVideo(filmVideo, reducedMotion ? "metadata" : "auto");
          filmNearObserver.unobserve(filmVideo);
        }
      },
      { rootMargin: "700px 0px" },
    );
    filmNearObserver.observe(filmVideo);
    cleanups.push(() => filmNearObserver.disconnect());
  }

  // 2. 播放控制 Observer：露出视口才播放，滚走立刻暂停
  if (!reducedMotion) {
    const cardPlayObserver = new win.IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            hydrateVideo(e.target, "auto");
            e.target.play().catch(() => {});
          } else {
            e.target.pause();
          }
        }
      },
      { threshold: 0.5 },
    );
    cardVideos.forEach((v) => cardPlayObserver.observe(v));
    cleanups.push(() => cardPlayObserver.disconnect());

    if (filmVideo) {
      const filmPlayObserver = new win.IntersectionObserver(
        (entries) => {
          for (const e of entries) {
            if (e.isIntersecting) {
              hydrateVideo(filmVideo, "auto");
              filmVideo.play().catch(() => {});
            } else {
              filmVideo.pause();
            }
          }
        },
        { threshold: 0.25 },
      );
      filmPlayObserver.observe(filmVideo);
      cleanups.push(() => filmPlayObserver.disconnect());
    }
  }

  // 3. 顺序接力队列：首屏完成后按从上到下顺序静默预热，不与首屏 Demo 抢带宽
  async function runCascade() {
    if (cascadeStarted || disposed || saveData) return;
    cascadeStarted = true;

    // P1a：先挂首批可视卡片（前 4 张）的轻量 webp 封面
    for (const v of cardVideos.slice(0, 4)) {
      if (disposed) return;
      hydratePoster(v);
    }
    await nextFrame();

    // P1b：串行预热前 3 张可见卡片的短视频（单条 80–380 KB），逐个接力避免并发挤占
    if (!reducedMotion) {
      for (const v of cardVideos.slice(0, 3)) {
        if (disposed) return;
        hydrateVideo(v, "auto");
        await waitLoadedOrTimeout(v, 700);
      }
    }

    // P2a：补齐右侧其余卡片的封面与底部大视频封面（133 KB）
    for (const v of cardVideos.slice(4)) {
      if (disposed) return;
      hydratePoster(v);
    }
    if (filmVideo && !disposed) {
      hydratePoster(filmVideo);
      // 停在首屏时不提前拉 11 MB 大视频（避免只看首屏的移动端用户白耗 1–2 MB 流量）；
      // 一旦检测到用户开始向下滚动，由下方 onScroll 立即预热 metadata
    }
  }

  function scheduleCascadeAfterHero(delayMs) {
    const timer = win.setTimeout(() => {
      if (disposed) return;
      if (typeof win.requestIdleCallback === "function") {
        const idleId = win.requestIdleCallback(() => void runCascade(), { timeout: 1000 });
        cleanups.push(() => win.cancelIdleCallback?.(idleId));
      } else {
        void runCascade();
      }
    }, delayMs);
    cleanups.push(() => win.clearTimeout(timer));
  }

  // 闸门 A：顶部交互 Demo iframe 加载完毕后稍等 500ms（给 demo 内动态语言包和首屏图片留出带宽，兜底 2500ms）
  const demoFrame = doc.querySelector(".hero-demo iframe");
  if (demoFrame) {
    const onDemoLoad = () => scheduleCascadeAfterHero(500);
    demoFrame.addEventListener("load", onDemoLoad, { once: true });
    cleanups.push(() => demoFrame.removeEventListener("load", onDemoLoad));
    scheduleCascadeAfterHero(2500);
  } else {
    scheduleCascadeAfterHero(300);
  }

  // 闸门 B：用户开始往下滚动或点击下方锚点，立刻开闸，并提前预热底部大视频 metadata
  const onScroll = () => {
    if ((win.scrollY || 0) <= 40) return;
    win.removeEventListener("scroll", onScroll);
    void runCascade();
    if (filmVideo && !saveData) {
      hydrateVideo(filmVideo, "metadata");
    }
  };
  win.addEventListener("scroll", onScroll, { passive: true });
  cleanups.push(() => win.removeEventListener("scroll", onScroll));

  const onAnchorClick = (e) => {
    const link = e.target && typeof e.target.closest === "function" ? e.target.closest("a[href]") : null;
    const href = link?.getAttribute("href");
    if (href === "#features" || href === "#demo") {
      void runCascade();
      if (href === "#demo" && filmVideo) {
        hydrateVideo(filmVideo, reducedMotion ? "metadata" : "auto");
      }
    }
  };
  doc.addEventListener("click", onAnchorClick);
  cleanups.push(() => doc.removeEventListener("click", onAnchorClick));

  return () => {
    disposed = true;
    cleanups.forEach((fn) => fn());
  };
}

if (typeof document !== "undefined" && typeof window !== "undefined") {
  initHomeMedia(document, window);
}
