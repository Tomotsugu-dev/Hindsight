import { describe, expect, it } from "vitest";
import { themeForSite } from "./site-mode";

describe("themeForSite", () => {
  it("uses the visitor's choice on the site over the system", () => {
    expect(themeForSite("dark", false)).toBe("dark");
    expect(themeForSite("light", true)).toBe("minimal");
  });

  it("follows the system when the visitor has not chosen", () => {
    expect(themeForSite(null, true)).toBe("dark");
    expect(themeForSite(null, false)).toBe("minimal");

    // 不认识的值当没选过
    expect(themeForSite("sepia", true)).toBe("dark");
  });
});
