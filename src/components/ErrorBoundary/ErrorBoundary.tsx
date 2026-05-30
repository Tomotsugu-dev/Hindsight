import { Component, type ErrorInfo, type ReactNode } from "react";
import { logError } from "../../lib/logger";
import { ErrorFallback } from "./ErrorFallback";

interface Props {
  children: ReactNode;
  /** 自定义兜底 UI；不传用默认 ErrorFallback */
  fallback?: ReactNode;
  /** 错误日志 scope，便于区分是顶层还是某一页边界 */
  scope?: string;
}

interface State {
  hasError: boolean;
}

/**
 * 渲染期异常边界。任意子树抛错时捕获并渲染兜底 UI，避免整窗白屏。
 * - 顶层包在 main.tsx 的 <App/> 外：保住整个应用不被单点崩溃带走
 * - 页面级包在 AppLayout 的 <Outlet/> 外（key=路由）：单页崩溃仍保留侧栏/窗口 chrome，
 *   切到别的页即可恢复
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false };

  static getDerivedStateFromError(): State {
    return { hasError: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    const scope = this.props.scope ?? "app.crash";
    logError(scope, error);
    // 组件栈对定位崩溃位置很关键，单独再记一条
    logError(`${scope}.componentStack`, info.componentStack);
  }

  render() {
    if (this.state.hasError) {
      return this.props.fallback ?? <ErrorFallback />;
    }
    return this.props.children;
  }
}
