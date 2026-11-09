import { renderToStaticMarkup } from "react-dom/server";
import { Search } from "lucide-react";
import { describe, expect, it } from "vitest";
import { TextInput } from "./TextInput";

describe("TextInput", () => {
  it("外框带 data-part 和尺寸，主题靠它们认出输入框", () => {
    const html = renderToStaticMarkup(<TextInput aria-label="搜索" />);
    expect(html).toMatch(/^<label[^>]*data-part="input"[^>]*data-size="md"/);
    expect(html).toContain('type="text"');
  });

  it("className 给外框，其余属性给 input", () => {
    const html = renderToStaticMarkup(
      <TextInput className="wide" placeholder="搜索插件" icon={Search} />,
    );
    expect(html).toMatch(/^<label class="[^"]*wide"/);
    expect(html).toMatch(/<input placeholder="搜索插件"/);
    // 图标在 input 前面，和它并排
    expect(html.indexOf("lucide-search")).toBeLessThan(html.indexOf("<input"));
  });
});
