// 用真的中文文案渲染隐私页，断言写成用户看到的样子
import i18next from "i18next";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider, initReactI18next } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeAll, describe, expect, it, vi } from "vitest";
import zhCN from "../../../i18n/locales/zh-CN.json";

// node 环境没有 Tauri；设置也不从后端读，直接给一份空的关键词
vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.resolve(null),
}));
vi.mock("../../../state/settings", () => ({
  useSettings: () => ({
    settings: { privacyUrlKeywords: [], privacyAppKeywords: [], recordBrowserHost: true },
    update: () => undefined,
  }),
}));

import PrivacyTab from "./PrivacyTab";

const i18n = i18next.createInstance();
beforeAll(async () => {
  await i18n.use(initReactI18next).init({
    lng: "zh-CN",
    resources: { "zh-CN": { translation: zhCN } },
  });
});

function render(): string {
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter>
        <PrivacyTab />
      </MemoryRouter>
    </I18nextProvider>,
  );
}

describe("PrivacyTab", () => {
  it("links to the Categories tabs where apps and websites are made private", () => {
    const html = render();
    expect(html).toContain('href="/categories/apps"');
    expect(html).toContain('href="/categories/sites"');
  });

  it("links to the Data tab of Settings where screenshots are cleared", () => {
    expect(render()).toContain('href="/settings/data"');
  });

  it("shows recording nothing before skipping screenshots", () => {
    // 前后带尖括号：只认整段就是这几个字的标题，不认气泡里顺带提到的
    const html = render();
    const nothingAt = html.indexOf(">完全不记录<");
    const screenshotsAt = html.indexOf(">只跳过截图<");
    expect(nothingAt).toBeGreaterThanOrEqual(0);
    expect(screenshotsAt).toBeGreaterThan(nothingAt);
  });
});
