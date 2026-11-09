import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Checkbox } from "./Checkbox";

describe("Checkbox", () => {
  it("label 带 data-part，勾选框在文字前面，点文字也能勾", () => {
    const html = renderToStaticMarkup(<Checkbox defaultChecked>同时删除数据</Checkbox>);
    expect(html).toMatch(/^<label[^>]*data-part="checkbox"/);
    expect(html).toMatch(/<input[^>]*type="checkbox"[^>]*><span>同时删除数据<\/span><\/label>$/);
    expect(html).toContain("checked");
  });

  it("没有文字时只有勾选框，不多一个空 span", () => {
    const html = renderToStaticMarkup(<Checkbox aria-label="启用" />);
    expect(html).not.toContain("<span>");
    expect(html).toContain('aria-label="启用"');
  });
});
