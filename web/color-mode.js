// 官网各页的深浅色：写在 <html data-mode> 上，site.css 按它换配色。
// 访客没选过就跟随系统。点顶栏的 .mode-toggle 切换；切到和系统一样的颜色时忘掉选择，重新跟随系统。
// 在 <head> 里同步加载：画页面之前就要设好，否则深色系统上先闪一下白。
(function () {
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
  }

  apply();
  // 第一次 apply 时按钮还没解析到
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
