import { renderToStaticMarkup } from "react-dom/server";
import { Trash2 } from "lucide-react";
import { describe, expect, it } from "vitest";
import { Button } from "./Button";
import { IconButton } from "./IconButton";

describe("Button", () => {
  it("默认是 soft、neutral、md，type 是 button，主题靠这几个属性认出它", () => {
    const html = renderToStaticMarkup(<Button>保存</Button>);
    expect(html).toContain('data-part="button"');
    expect(html).toContain('data-variant="soft"');
    expect(html).toContain('data-tone="neutral"');
    expect(html).toContain('data-size="md"');
    // 默认 submit 的话，放在表单里会意外提交
    expect(html).toContain('type="button"');
  });

  it("loading 时不能点，标上 aria-busy，图标换成转圈", () => {
    const html = renderToStaticMarkup(
      <Button loading icon={Trash2}>
        卸载
      </Button>,
    );
    expect(html).toContain("disabled");
    expect(html).toContain('aria-busy="true"');
    expect(html).toContain("lucide-loader");
    expect(html).not.toContain("lucide-trash");
  });

  it("className 加在后面，只管位置，样式属性照旧", () => {
    const html = renderToStaticMarkup(
      <Button variant="solid" tone="danger" className="pushRight">
        删除
      </Button>,
    );
    expect(html).toMatch(/class="[^"]*pushRight"/);
    expect(html).toContain('data-variant="solid"');
    expect(html).toContain('data-tone="danger"');
  });

  it("glow 时带上全局的 glow 类，鼠标靠近时描边发光；默认没有", () => {
    expect(renderToStaticMarkup(<Button glow>今天</Button>)).toMatch(/class="[^"]*\bglow\b/);
    expect(renderToStaticMarkup(<Button>今天</Button>)).not.toMatch(/\bglow\b/);
  });
});

describe("IconButton", () => {
  it("没有文字，label 给读屏和悬停提示，默认 ghost", () => {
    const html = renderToStaticMarkup(<IconButton icon={Trash2} label="删除" />);
    expect(html).toContain('aria-label="删除"');
    expect(html).toContain('title="删除"');
    expect(html).toContain('data-variant="ghost"');
    expect(html).not.toMatch(/<\/svg>[^<]+<\/button>/);
  });
});
