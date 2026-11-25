import { expect, it, vi } from "vitest";

// 网页里打不开访客电脑上的文件和文件夹。这些按钮点了要说一声，不能没反应
it("tells the visitor that files and folders need the desktop app", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  const alert = vi.fn();
  vi.stubGlobal("window", { alert });
  const i18n = (await import("@app/i18n")).default;
  await i18n.changeLanguage("en");
  const dialog = await import("./tauri/plugin-dialog");
  const opener = await import("./tauri/plugin-opener");
  const { invoke } = await import("./tauri/core");
  const { api } = await import("./api-mock");

  expect(await dialog.open({ directory: true })).toBeNull();
  await opener.openPath("C:\\Users\\demo");
  await opener.revealItemInDir("C:\\Users\\demo\\hindsight.db");
  await invoke("theme_open_folder");
  await invoke("plugin_open_zips_folder");
  await api.openScreenshotsDir();

  expect(alert).toHaveBeenCalledTimes(6);
  expect(alert.mock.calls[0][0]).toMatch(/desktop app/);
  await i18n.changeLanguage("ja");
  await opener.openPath("C:\\Users\\demo");
  expect(alert.mock.calls[6][0]).toMatch(/デスクトップ版/);
  await i18n.changeLanguage("zh-CN");
  vi.unstubAllGlobals();
});
