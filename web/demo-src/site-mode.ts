// demo 嵌在官网首页里。访客没在 demo 里选过主题时，跟着官网的深浅色用深色或简约主题。
// 官网的深浅色由 web/color-mode.js 存在 localStorage，没存时跟随系统。
import { hasStoredTheme, showTheme, type AppTheme } from "@app/lib/theme";

// 和 web/color-mode.js 的一样
const SITE_KEY = "hindsight.site.mode";

/** 返回和官网深浅色对应的主题。stored 是官网存的值，systemDark 是系统是不是深色。 */
export function themeForSite(stored: string | null, systemDark: boolean): AppTheme {
  const dark = stored === "dark" || (stored !== "light" && systemDark);
  return dark ? "dark" : "minimal";
}

/** 现在套用一次；之后官网或系统换深浅色时跟着换。 */
export function followSiteMode(): void {
  const system = matchMedia("(prefers-color-scheme: dark)");
  const follow = () => {
    if (hasStoredTheme()) return;
    showTheme(themeForSite(localStorage.getItem(SITE_KEY), system.matches));
  };
  follow();
  system.addEventListener("change", follow);
  // 首页上点了深浅色按钮。iframe 和首页同源，首页写 localStorage 时这里会收到
  addEventListener("storage", (e) => {
    if (e.key === SITE_KEY) follow();
  });
}
