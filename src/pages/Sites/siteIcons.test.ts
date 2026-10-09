import { describe, expect, it, vi } from "vitest";
import type { SiteIconDownloadRound, SiteIconPaths } from "../../api/hindsight";
import { createSiteIconLoader } from "./siteIcons";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function request(overrides = {}) {
  return {
    hosts: ["github.com", "example.com"],
    enabled: true,
    isActive: () => true,
    onIcons: vi.fn<(icons: SiteIconPaths) => void>(),
    ...overrides,
  };
}

describe("website icon loading", () => {
  it("shows cached icons with downloads disabled and never requests websites", async () => {
    const cached = { "github.com": "/cache/github.com.png" };
    const backend = {
      getSiteIcons: vi.fn().mockResolvedValue(cached),
      downloadSiteIcons: vi.fn(),
    };
    const job = request({ enabled: false });

    await createSiteIconLoader(backend)(job);

    expect(job.onIcons).toHaveBeenCalledWith(cached);
    expect(backend.downloadSiteIcons).not.toHaveBeenCalled();
  });

  it("keeps cached icons and merges batches in the supplied website order", async () => {
    const cached = { "cached.com": "/cache/cached.com.png" };
    const first = { "github.com": "/cache/github.com.png" };
    const second = { "example.com": "/cache/example.com.png" };
    const backend = {
      getSiteIcons: vi.fn().mockResolvedValue(cached),
      downloadSiteIcons: vi
        .fn()
        .mockResolvedValueOnce({ icons: first, remaining: 1 })
        .mockResolvedValueOnce({ icons: second, remaining: 0 }),
    };
    const job = request();

    await createSiteIconLoader(backend)(job);

    expect(backend.downloadSiteIcons.mock.calls).toEqual([[job.hosts], [job.hosts]]);
    expect(job.onIcons.mock.calls.map(([icons]) => icons)).toEqual([
      cached,
      { ...cached, ...first },
      { ...cached, ...first, ...second },
    ]);
  });

  it("stops after the current batch when the page closes or downloads are disabled", async () => {
    const batch = deferred<SiteIconDownloadRound>();
    let active = true;
    const backend = {
      getSiteIcons: vi.fn().mockResolvedValue({}),
      downloadSiteIcons: vi.fn().mockReturnValue(batch.promise),
    };
    const job = request({ isActive: () => active });
    const task = createSiteIconLoader(backend)(job);
    await vi.waitFor(() => expect(backend.downloadSiteIcons).toHaveBeenCalledOnce());

    active = false;
    batch.resolve({ icons: { "github.com": "/cache/github.com.png" }, remaining: 5 });
    await task;

    expect(backend.downloadSiteIcons).toHaveBeenCalledOnce();
    expect(job.onIcons).toHaveBeenCalledOnce();
  });

  it("serializes a reopened page behind the unfinished batch and reads its cache afterward", async () => {
    const batch = deferred<SiteIconDownloadRound>();
    let firstActive = true;
    const backend = {
      getSiteIcons: vi
        .fn()
        .mockResolvedValueOnce({})
        .mockResolvedValueOnce({ "github.com": "/cache/github.com.png" }),
      downloadSiteIcons: vi.fn().mockReturnValue(batch.promise),
    };
    const load = createSiteIconLoader(backend);
    const first = load(request({ isActive: () => firstActive }));
    await vi.waitFor(() => expect(backend.downloadSiteIcons).toHaveBeenCalledOnce());
    firstActive = false;
    const reopened = request({ enabled: false });
    const second = load(reopened);
    await Promise.resolve();

    expect(backend.getSiteIcons).toHaveBeenCalledOnce();
    batch.resolve({ icons: {}, remaining: 3 });
    await Promise.all([first, second]);

    expect(backend.getSiteIcons).toHaveBeenCalledTimes(2);
    expect(backend.downloadSiteIcons).toHaveBeenCalledOnce();
    expect(reopened.onIcons).toHaveBeenCalledWith({ "github.com": "/cache/github.com.png" });
  });

  it("skips a request canceled before it starts", async () => {
    const backend = { getSiteIcons: vi.fn(), downloadSiteIcons: vi.fn() };
    await createSiteIconLoader(backend)(request({ isActive: () => false }));
    expect(backend.getSiteIcons).not.toHaveBeenCalled();
    expect(backend.downloadSiteIcons).not.toHaveBeenCalled();
  });

  it("does not start downloads if canceled while reading cached icons", async () => {
    const cached = deferred<Record<string, string>>();
    let active = true;
    const backend = {
      getSiteIcons: vi.fn().mockReturnValue(cached.promise),
      downloadSiteIcons: vi.fn(),
    };
    const job = request({ isActive: () => active });
    const task = createSiteIconLoader(backend)(job);
    await vi.waitFor(() => expect(backend.getSiteIcons).toHaveBeenCalledOnce());
    active = false;
    cached.resolve({});
    await task;
    expect(job.onIcons).not.toHaveBeenCalled();
    expect(backend.downloadSiteIcons).not.toHaveBeenCalled();
  });

  it("stops instead of repeatedly downloading a batch that makes no progress", async () => {
    const backend = {
      getSiteIcons: vi.fn().mockResolvedValue({}),
      downloadSiteIcons: vi.fn().mockResolvedValue({ icons: {}, remaining: 3 }),
    };
    await expect(createSiteIconLoader(backend)(request())).rejects.toThrow("no progress");
    expect(backend.downloadSiteIcons).toHaveBeenCalledTimes(2);
  });

  it("can retry after an earlier cache request failed", async () => {
    const backend = {
      getSiteIcons: vi
        .fn()
        .mockRejectedValueOnce(new Error("cache unavailable"))
        .mockResolvedValueOnce({}),
      downloadSiteIcons: vi.fn(),
    };
    const load = createSiteIconLoader(backend);
    await expect(load(request({ enabled: false }))).rejects.toThrow("cache unavailable");
    await load(request({ enabled: false }));
    expect(backend.getSiteIcons).toHaveBeenCalledTimes(2);
    expect(backend.downloadSiteIcons).not.toHaveBeenCalled();
  });

  it("reads the cache for an empty list without starting downloads", async () => {
    const backend = {
      getSiteIcons: vi.fn().mockResolvedValue({}),
      downloadSiteIcons: vi.fn(),
    };
    await createSiteIconLoader(backend)(request({ hosts: [] }));
    expect(backend.getSiteIcons).toHaveBeenCalledOnce();
    expect(backend.downloadSiteIcons).not.toHaveBeenCalled();
  });
});
