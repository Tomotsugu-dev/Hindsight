// 官网各页的深浅色：写在 <html data-mode> 上，site.css 按它换配色。
// 访客没选过就跟随系统。点顶栏的 .mode-toggle 切换；切到和系统一样的颜色时忘掉选择，重新跟随系统。
// 在 <head> 里同步加载：画页面之前就要设好，否则深色系统上先闪一下白。
(function () {
  // web/demo-src/site-mode.ts 也读这个键，让首页里的 demo 跟着变
  var KEY = "hindsight.site.mode";
  var root = document.documentElement;
  var system = matchMedia("(prefers-color-scheme: dark)");
  var chosen = read();

  function read() {
    try {
      var value = localStorage.getItem(KEY);
      return value === "light" || value === "dark" ? value : null;
    } catch (e) {
      return null;
    }
  }

  function systemMode() {
    return system.matches ? "dark" : "light";
  }

  function apply() {
    var mode = chosen || systemMode();
    root.dataset.mode = mode;
    document.querySelectorAll(".mode-toggle").forEach(function (button) {
      button.setAttribute("aria-pressed", String(mode === "dark"));
    });
    document.querySelectorAll("video[data-dark-src]").forEach(function (video) {
      showVideo(video, mode === "dark");
    });
  }

  // 首页有深浅两版的视频：浅色版在 data-src / data-poster，深色版在 data-dark-src / data-dark-poster。
  // 什么时候挂上 src、poster 由 home-media.js 决定，它按当前颜色取；这里只把已经挂上的换成另一版。
  // 两版时间轴相同，换版时从原来的位置接着播，原来在播就接着播。
  // 跳到原来的位置要服务器支持 Range，见 worker.js
  function showVideo(video, dark) {
    if (video.hasAttribute("poster")) {
      video.setAttribute("poster", dark ? video.dataset.darkPoster : video.dataset.poster);
    }
    var src = dark ? video.dataset.darkSrc : video.dataset.src;
    if (!video.hasAttribute("src") || video.getAttribute("src") === src) return;
    var at = video.currentTime;
    var playing = !video.paused;
    video.setAttribute("src", src);
    if (at > 0) {
      video.addEventListener("loadedmetadata", function () {
        video.currentTime = at;
      }, { once: true });
    }
    if (playing) {
      var started = video.play();
      if (started) started.catch(function () {});
    }
  }

  apply();
  // 第一次 apply 时按钮和视频还没解析到
  document.addEventListener("DOMContentLoaded", apply);
  system.addEventListener("change", apply);
  // 别的标签页切换了
  addEventListener("storage", function (e) {
    if (e.key !== KEY) return;
    chosen = read();
    apply();
  });

  document.addEventListener("click", function (e) {
    if (!(e.target instanceof Element) || !e.target.closest(".mode-toggle")) return;
    var next = (chosen || systemMode()) === "dark" ? "light" : "dark";
    chosen = next === systemMode() ? null : next;
    try {
      if (chosen) localStorage.setItem(KEY, chosen);
      else localStorage.removeItem(KEY);
    } catch (err) {
      // 存不了时只在这一页生效
    }
    apply();
  });
})();
