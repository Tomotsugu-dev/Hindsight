// 用真的中文文案渲染，断言写成用户看到的样子
import i18next from "i18next";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider, initReactI18next } from "react-i18next";
import { beforeAll, expect, it, vi } from "vitest";
import zhCN from "../../i18n/locales/zh-CN.json";

// 真的 i18n 入口一加载就读 localStorage，node 环境没有；设置也不从后端读
vi.mock("../../i18n/index", () => ({ LOCALE_STORAGE_KEY: "locale" }));
vi.mock("../../state/settings", () => ({ useSettings: () => ({ settings: null }) }));

import { StatusFooter } from "./StatusFooter";

const i18n = i18next.createInstance();
beforeAll(async () => {
  await i18n.use(initReactI18next).init({
    lng: "zh-CN",
    resources: { "zh-CN": { translation: zhCN } },
  });
});

// 语言按钮的提示也是它的名字（按钮里的字对读屏隐藏），要用界面的语言说
it("names the language button in the page language", () => {
  const html = renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <StatusFooter />
    </I18nextProvider>,
  );
  expect(html).toContain('title="切换到「繁體中文」"');
});
