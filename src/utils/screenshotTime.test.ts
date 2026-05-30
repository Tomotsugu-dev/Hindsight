import { describe, it, expect } from "vitest";
import { extractScreenshotTime } from "./screenshotTime";

describe("extractScreenshotTime", () => {
  it("从 HHMMSS_NNN.jpg 取出 HH:MM", () => {
    expect(extractScreenshotTime("/data/shots/143005_271.jpg")).toBe("14:30");
  });

  it("支持 Windows 反斜杠路径", () => {
    expect(extractScreenshotTime("C:\\shots\\090000_001.jpg")).toBe("09:00");
  });

  it("午夜 000000 → 00:00", () => {
    expect(extractScreenshotTime("000000_999.jpg")).toBe("00:00");
  });

  it("纯文件名（无目录）也能解析", () => {
    expect(extractScreenshotTime("235959_010.png")).toBe("23:59");
  });

  it("时间段长度不是 6 位 → ??:??", () => {
    expect(extractScreenshotTime("12345_001.jpg")).toBe("??:??");
  });

  it("时间段含非数字 → ??:??", () => {
    expect(extractScreenshotTime("12a405_001.jpg")).toBe("??:??");
  });

  it("没有下划线分隔的文件名 → ??:??", () => {
    expect(extractScreenshotTime("randomname.jpg")).toBe("??:??");
  });

  it("空路径 → ??:??", () => {
    expect(extractScreenshotTime("")).toBe("??:??");
  });
});
