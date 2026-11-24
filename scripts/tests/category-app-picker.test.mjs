import assert from "node:assert/strict";
import { test } from "node:test";
import { demoOptions, withDemo } from "../shots/web-demo.mjs";

test("a category stays expanded between consecutive process assignments", async () => {
  const options = await demoOptions({ lang: "en", theme: "default" });
  await withDemo(options, async ({ page, labels }) => {
    await page.evaluate(() => {
      const category = {
        id: "picker-test",
        name: "Picker test",
        color: "#a78bfa",
        icon: "Code",
        builtin: false,
        superCategoryId: "work",
        apps: Array.from({ length: 20 }, (_, index) => `App ${index}.exe`),
      };
      localStorage.setItem("hindsight-demo-state-v6", JSON.stringify({ categories: [category] }));
    });
    await page.reload();
    await page.getByRole("link", { name: labels.nav.items.categories, exact: true }).click();
    const category = page.getByRole("button", { name: "Picker test", exact: true });
    await category.waitFor();
    const row = category.locator("xpath=../../..");
    const add = page.getByRole("button", { name: "Add app", exact: true });
    const input = page.getByPlaceholder("e.g. chrome.exe");
    await add.hover();
    await add.click();
    await input.fill("firefox");
    const option = page.getByRole("option", { name: /firefox/i });
    const optionBox = await option.boundingBox();
    assert.ok(optionBox);
    const pointer = { x: optionBox.x + 20, y: optionBox.y + optionBox.height - 4 };
    await page.mouse.click(pointer.x, pointer.y);
    await page.getByRole("button", { name: "Remove firefox", exact: true }).waitFor();
    const rowBox = await row.boundingBox();
    assert.ok(rowBox);
    assert.ok(
      pointer.x < rowBox.x ||
        pointer.x >= rowBox.x + rowBox.width ||
        pointer.y < rowBox.y ||
        pointer.y >= rowBox.y + rowBox.height,
      "the stationary pointer should be outside the category row after the assignment",
    );
    assert.equal(await input.evaluateAll(([field]) => field === document.activeElement), true);
    const clip = await input.locator("xpath=../../..").elementHandle();
    await page.waitForFunction(
      (element) => element.clientHeight >= element.firstElementChild.scrollHeight,
      clip,
    );

    await page.keyboard.type("second-test.exe");
    await page.keyboard.press("Enter");
    await page.getByRole("button", { name: "Remove second-test", exact: true }).waitFor();
    await page.keyboard.press("Escape");
    await add.waitFor();
    await page.waitForFunction(
      (element) => element.clientHeight < element.firstElementChild.scrollHeight,
      clip,
    );
  });
});
