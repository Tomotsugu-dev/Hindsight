// 用真的中文文案渲染，断言写成用户看到的样子
import i18next from "i18next";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider, initReactI18next } from "react-i18next";
import { beforeAll, expect, it } from "vitest";
import zhCN from "../../i18n/locales/zh-CN.json";
import { ViewToggle } from "./ViewToggle";

const i18n = i18next.createInstance();
beforeAll(async () => {
  await i18n.use(initReactI18next).init({
    lng: "zh-CN",
    resources: { "zh-CN": { translation: zhCN } },
  });
});

function render(bar: "hour" | "day"): string {
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <ViewToggle view="bars" bar={bar} onChange={() => {}} />
    </I18nextProvider>,
  );
}

// 日统计的柱子一根是一小时；周、月的柱子一根是一天，不能也写「时段」
it("names the bar view after what one bar stands for", () => {
  expect(render("hour")).toContain("<span>时段</span>");
  expect(render("day")).toContain("<span>每日</span>");
  expect(render("day")).toContain('aria-label="切换到每日分布视图"');
});
