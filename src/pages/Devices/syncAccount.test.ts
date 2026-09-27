import { describe, expect, it } from "vitest";
import type { AuthState } from "../../api/hindsight";
import {
  canClearCloud,
  connectErrorKind,
  serviceKind,
  webdavAccountLabel,
} from "./syncAccount";

const base: AuthState = {
  backend: "drive",
  signedIn: true,
  uid: "g-1",
  email: "a@example.com",
  configured: true,
  webdavUrl: null,
  webdavUser: null,
};

describe("webdavAccountLabel", () => {
  it("显示用户名和服务器", () => {
    expect(
      webdavAccountLabel("https://dav.jianguoyun.com/dav/", "you@example.com"),
    ).toBe("you@example.com @ dav.jianguoyun.com");
  });

  it("地址解析不了就原样显示", () => {
    expect(webdavAccountLabel("not a url", "me")).toBe("me @ not a url");
  });

  it("没有用户名只显示服务器", () => {
    expect(webdavAccountLabel("https://nc.example.com/remote.php/dav/", null)).toBe(
      "nc.example.com",
    );
  });
});

describe("connectErrorKind", () => {
  it("认出后端的三种错误", () => {
    expect(connectErrorKind("webdav propfind returned 401: Unauthorized")).toBe(
      "wrongPassword",
    );
    expect(connectErrorKind("invalid input: WebDAV URL must be https")).toBe("notHttps");
    expect(
      connectErrorKind("invalid input: WebDAV URL is invalid: relative URL without a base"),
    ).toBe("invalidUrl");
  });

  it("其他错误原样显示", () => {
    expect(connectErrorKind("http: error sending request")).toBe("other");
    expect(connectErrorKind("webdav propfind returned 4010: x")).toBe("other");
  });
});

describe("serviceKind", () => {
  const onWebdav = (webdavUrl: string | null): AuthState => ({
    ...base,
    backend: "webdav",
    webdavUrl,
    webdavUser: "you@example.com",
  });

  it("Drive 是 Google Drive", () => {
    expect(serviceKind(base)).toBe("googleDrive");
    expect(serviceKind(null)).toBe("googleDrive");
  });

  it("坚果云的地址认成坚果云，不管路径和大小写", () => {
    expect(serviceKind(onWebdav("https://dav.jianguoyun.com/dav/"))).toBe("nutstore");
    expect(serviceKind(onWebdav("https://DAV.jianguoyun.com/dav/hindsight"))).toBe(
      "nutstore",
    );
  });

  it("其他服务器和解析不了的地址都是 WebDAV", () => {
    expect(serviceKind(onWebdav("https://nc.example.com/remote.php/dav/"))).toBe("webdav");
    expect(serviceKind(onWebdav("not a url"))).toBe("webdav");
    expect(serviceKind(onWebdav(null))).toBe("webdav");
  });
});

describe("canClearCloud", () => {
  it("登着 Google Drive 才能清", () => {
    expect(canClearCloud(base)).toBe(true);
    expect(canClearCloud({ ...base, signedIn: false })).toBe(false);
    expect(canClearCloud(null)).toBe(false);
  });

  it("WebDAV 上不能清", () => {
    expect(
      canClearCloud({
        ...base,
        backend: "webdav",
        uid: null,
        email: null,
        webdavUrl: "https://dav.jianguoyun.com/dav/",
        webdavUser: "you@example.com",
      }),
    ).toBe(false);
  });
});
