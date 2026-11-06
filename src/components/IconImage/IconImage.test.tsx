import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { IconImage } from "./IconImage";

describe("IconImage", () => {
  it("loads the image with crossOrigin anonymous", () => {
    // 不带这个属性，画布读像素会报 SecurityError
    const html = renderToStaticMarkup(<IconImage src="asset://localhost/a.png" size={16} alt="" />);
    expect(html).toContain('crossorigin="anonymous"');
  });
});
