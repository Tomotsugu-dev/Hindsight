// 按 ?lang= 设置 demo 的界面语言。i18n 启动时从 localStorage 读语言，所以 main-demo.tsx 第一个
// import 这个文件，让它比 i18n 先运行。不写成 index.html 里的内联脚本：/demo/ 的 CSP 不允许。

// 应用的六种界面语言（src/i18n/index.ts）
const LANGUAGES = ["zh-CN", "zh-TW", "en", "ja", "pt-BR", "es"];

try {
  const lng = new URLSearchParams(location.search).get("lang");
  if (lng !== null && LANGUAGES.includes(lng)) {
    localStorage.setItem("hindsight.locale", lng);
  }
} catch {
  // 隐私模式等情况下 localStorage 用不了：用默认语言
}

// 让 TypeScript 把这个文件当成模块
export {};
