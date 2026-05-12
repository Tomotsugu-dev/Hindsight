// Mock 替换 @tauri-apps/api/window
//
// 主应用用 getCurrentWindow() 控制窗口（最小化、最大化、关闭、拖动）。
// Demo 在 iframe 里跑，这些操作没意义，全部 no-op。

class MockWindow {
  label = "main";
  async minimize(): Promise<void> {}
  async maximize(): Promise<void> {}
  async unmaximize(): Promise<void> {}
  async toggleMaximize(): Promise<void> {}
  async close(): Promise<void> {}
  async hide(): Promise<void> {}
  async show(): Promise<void> {}
  async setFocus(): Promise<void> {}
  async isMaximized(): Promise<boolean> {
    return false;
  }
  async isMinimized(): Promise<boolean> {
    return false;
  }
  async startDragging(): Promise<void> {}
  async listen<T = unknown>(_event: string, _handler: (e: T) => void): Promise<() => void> {
    return () => {};
  }
  async onResized(_handler: (e: unknown) => void): Promise<() => void> {
    return () => {};
  }
  async onMoved(_handler: (e: unknown) => void): Promise<() => void> {
    return () => {};
  }
}

const stub = new MockWindow();

export function getCurrentWindow(): MockWindow {
  return stub;
}

export function getCurrent(): MockWindow {
  return stub;
}

export const Window = MockWindow;
