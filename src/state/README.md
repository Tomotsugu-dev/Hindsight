# `src/state/`

跨页面的应用状态。**两种风格**，按文件后缀区分：

| 后缀 | 含义 | 例子 |
|---|---|---|
| `.tsx` | React Context Provider — `<XxxProvider>` 包住整棵树，子树用 `useXxx()` 拿状态 | `settings.tsx`, `categories.tsx`, `deviceFilter.tsx`, `updater.tsx` |
| `.ts` | 模块级 store — 全局 listener + `useSyncExternalStore` 订阅，不占 Provider 树 | `dailySummary.ts`, `modelDownloads.ts` |

**何时选哪种**：
- 状态本身需要 SSR-safe 的初始化、跟着组件挂载/卸载、或天然嵌套于一个 React 子树 → Provider（`.tsx`）
- 状态是事件流（如 Tauri event listener）+ 跨整个 app 的全局值，且不需要 Provider 包装 → 模块级 store（`.ts`）。这样切侧栏 unmount 不会丢监听。

**错误处理**：所有这些文件的 catch 都走 `lib/logger.ts` 的 `logError(scope, err)`，scope 命名格式 `area.action`（如 `settings.load`、`devices.renameSelf`）。
