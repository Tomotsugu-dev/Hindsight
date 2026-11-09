import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PeriodNav } from "./PeriodNav";

const noop = () => {};
const base = {
  label: "今天",
  currentTitle: "回到今天",
  prevLabel: "前一天",
  nextLabel: "后一天",
  onPrev: noop,
  onNext: noop,
  onCurrent: noop,
};

const buttons = (html: string) => html.match(/<button[^>]*>/g) ?? [];

describe("PeriodNav", () => {
  it("在当前时段：中间和「›」点不了，中间不挂「回到今天」的提示", () => {
    const html = renderToStaticMarkup(<PeriodNav {...base} isCurrent canNext={false} />);
    const [prev, pill, next] = buttons(html);
    expect(prev).not.toContain("disabled");
    expect(pill).toContain("disabled");
    expect(pill).not.toContain("回到今天");
    expect(next).toContain("disabled");
  });

  it("翻到过去：中间能点回来，带提示", () => {
    const html = renderToStaticMarkup(
      <PeriodNav {...base} label="昨天" isCurrent={false} canNext />,
    );
    const [, pill, next] = buttons(html);
    expect(pill).not.toContain("disabled");
    expect(pill).toContain('title="回到今天"');
    expect(next).not.toContain("disabled");
  });

  it("busy 时三个都点不了", () => {
    const html = renderToStaticMarkup(<PeriodNav {...base} isCurrent={false} canNext busy />);
    expect(buttons(html).every((b) => b.includes("disabled"))).toBe(true);
  });
});
