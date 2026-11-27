// The main window's entry. It loads the UI state first and only then imports the rest of the app,
// because some modules read the UI state as they load (ADR-0020).
import { type } from "@tauri-apps/plugin-os";
import { logError } from "./lib/logger";
import { loadUiState } from "./state/loadUiState";

// 把当前 OS 写到 body[data-platform]，CSS 据此区分 macOS / windows / linux 的 chrome 表现
try {
  document.body.dataset.platform = type();
} catch {
  document.body.dataset.platform = "windows"; // 默认按 Windows 渲染
}

void loadUiState()
  .then(() => import("./renderApp"))
  .catch((e) => logError("main.start", e));
