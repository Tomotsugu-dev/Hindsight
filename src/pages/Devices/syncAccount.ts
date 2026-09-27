import type { AuthState } from "../../api/hindsight";

/** 坚果云的 WebDAV 地址，表单里当示例。 */
export const NUTSTORE_DAV_URL = "https://dav.jianguoyun.com/dav/";

/** 设备页上显示的服务名，对应 i18n 的 `devices.cloud.services.<kind>`。 */
export type ServiceKind = "googleDrive" | "nutstore" | "webdav";

/** 用的是哪家服务：WebDAV 地址是坚果云的就叫坚果云，其他 WebDAV 服务器统称 WebDAV。 */
export function serviceKind(auth: AuthState | null): ServiceKind {
  if (auth?.backend !== "webdav") return "googleDrive";
  try {
    if (auth.webdavUrl && new URL(auth.webdavUrl).host === new URL(NUTSTORE_DAV_URL).host) {
      return "nutstore";
    }
  } catch {
    // 地址解析不了，按一般的 WebDAV
  }
  return "webdav";
}

/** WebDAV 账号的显示名：「用户名 @ 服务器」；地址解析不了就原样显示。 */
export function webdavAccountLabel(url: string | null, user: string | null): string {
  let host = url ?? "";
  if (url) {
    try {
      host = new URL(url).host;
    } catch {
      // 保留原文
    }
  }
  return user ? `${user} @ ${host}` : host;
}

/** 连接 WebDAV 失败时给用户看哪句话；认不出的原样显示。 */
export type ConnectErrorKind =
  | "wrongPassword"
  | "serverBusy"
  | "notHttps"
  | "invalidUrl"
  | "other";

export function connectErrorKind(message: string): ConnectErrorKind {
  if (/returned 401\b/.test(message)) return "wrongPassword";
  // 429 / 503：云端限流或暂时不可用（坚果云限流回 503）。
  if (/returned (?:429|503)\b/.test(message)) return "serverBusy";
  if (message.includes("must be https")) return "notHttps";
  if (message.includes("URL is invalid")) return "invalidUrl";
  return "other";
}

/** 能不能从云端清数据：要登录着；WebDAV 上还不支持，后端也会拒绝。 */
export function canClearCloud(auth: AuthState | null): boolean {
  return !!auth && auth.signedIn && auth.backend !== "webdav";
}
