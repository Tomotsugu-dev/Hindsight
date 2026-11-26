import assert from "node:assert/strict";
import { test } from "node:test";
import { demoOptions, withDemo } from "../shots/web-demo.mjs";

// The first visit to Plugins removes its "New" badge, so the tab gets narrower while the route
// stays the same.
test("the tab highlight fits the Plugins tab after its badge goes away", async () => {
  const options = await demoOptions({ lang: "en", theme: "default" });
  await withDemo(options, async ({ page, labels }) => {
    await page.getByRole("link", { name: new RegExp(`^${labels.nav.items.settings}`) }).click();
    const tabs = page.getByRole("navigation", { name: labels.settings.pageTitle, exact: true });
    const badge = tabs.getByText(labels.plugins.newBadge, { exact: true });
    await badge.waitFor();

    await tabs.getByRole("link", { name: labels.settings.tabs.plugins }).click();
    await badge.waitFor({ state: "detached" });
    await tabs.evaluate(async (nav) => {
      const pill = nav.querySelector(":scope > [aria-hidden]");
      await new Promise((resolve) => requestAnimationFrame(resolve));
      await Promise.all(pill.getAnimations().map((a) => a.finished));
    });

    const { tab, pill } = await tabs.evaluate((nav) => {
      const box = (el) => {
        const r = el.getBoundingClientRect();
        return { left: Math.round(r.left), width: Math.round(r.width) };
      };
      return {
        tab: box(nav.querySelector('[aria-current="page"]')),
        pill: box(nav.querySelector(":scope > [aria-hidden]")),
      };
    });
    // TabNav places the pill with offsetLeft and offsetWidth, which are whole pixels.
    const off = Math.max(Math.abs(pill.left - tab.left), Math.abs(pill.width - tab.width));
    assert.ok(off <= 1, `pill ${JSON.stringify(pill)} does not fit tab ${JSON.stringify(tab)}`);
  });
});
