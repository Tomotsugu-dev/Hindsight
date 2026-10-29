import { describe, it, expect } from "vitest";
import { displayAppName } from "./displayName";

describe("displayAppName", () => {
  it("去掉 .exe 后缀", () => {
    expect(displayAppName("code.exe")).toBe("code");
  });

  it("去掉 .app / .lnk 后缀", () => {
    expect(displayAppName("Safari.app")).toBe("Safari");
    expect(displayAppName("Steam.lnk")).toBe("Steam");
  });

  it("后缀大小写不敏感", () => {
    expect(displayAppName("Game.EXE")).toBe("Game");
  });

  it("只去结尾的可执行后缀，不动名字中间的同名片段", () => {
    expect(displayAppName("my.exe.tool.exe")).toBe("my.exe.tool");
  });

  it("Linux 的反向域名应用 ID 只留最后一段", () => {
    expect(displayAppName("org.gnome.Nautilus")).toBe("Nautilus");
    expect(displayAppName("org.kde.konsole")).toBe("konsole");
    expect(displayAppName("com.microsoft.VSCode")).toBe("VSCode");
    expect(displayAppName("io.github.flattool.Warehouse")).toBe("Warehouse");
  });

  it("不像应用 ID 的带点名字原样返回", () => {
    // 只有两段
    expect(displayAppName("draw.io")).toBe("draw.io");
    // 第一段不是常见顶级域
    expect(displayAppName("my.exe.tool")).toBe("my.exe.tool");
  });

  it("无已知后缀时原样返回", () => {
    expect(displayAppName("chrome")).toBe("chrome");
    expect(displayAppName("notes.txt")).toBe("notes.txt");
  });

  it("空串原样返回", () => {
    expect(displayAppName("")).toBe("");
  });
});
