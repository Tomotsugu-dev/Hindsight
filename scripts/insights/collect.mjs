#!/usr/bin/env node
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { collectAll, createJsonClient, normalizeTrafficDays, redactError, trafficWindow, validateConfig } from "./lib.mjs";

const CONFIG = new URL("./config.json", import.meta.url);

export function githubCollector(token, request = createJsonClient()) {
  const headers = {
    Accept: "application/vnd.github+json",
    Authorization: `Bearer ${token}`,
    "X-GitHub-Api-Version": "2022-11-28",
    "User-Agent": "Hindsight-Internal-Insights",
  };
  const get = (endpoint) => request(`https://api.github.com${endpoint}`, { headers });
  const paginate = async (endpoint) => {
    const result = [];
    for (let page = 1; ; page++) {
      const rows = await get(`${endpoint}${endpoint.includes("?") ? "&" : "?"}per_page=100&page=${page}`);
      if (!Array.isArray(rows)) throw new Error(`Expected a list from GitHub ${endpoint}`);
      result.push(...rows);
      if (rows.length < 100) return result;
    }
  };
  return async (repository, now) => {
    const endpoint = `/repos/${repository}`;
    const [repo, views, clones, referrers, paths, releases] = await Promise.all([
      get(endpoint),
      get(`${endpoint}/traffic/views?per=day`),
      get(`${endpoint}/traffic/clones?per=day`),
      get(`${endpoint}/traffic/popular/referrers`),
      get(`${endpoint}/traffic/popular/paths`),
      paginate(`${endpoint}/releases`),
    ]);
    if (!Array.isArray(referrers) || !Array.isArray(paths)) throw new Error("Missing GitHub traffic source records");
    for (const field of ["stargazers_count", "forks_count", "subscribers_count", "open_issues_count"]) {
      if (!Number.isSafeInteger(repo?.[field]) || repo[field] < 0) throw new Error(`Missing repository statistic ${field}`);
    }
    const normalizedReleases = [];
    for (const release of releases.filter((release) => !release.draft)) {
      const assets = await paginate(`${endpoint}/releases/${release.id}/assets`);
      normalizedReleases.push({
        id: release.id,
        tag: release.tag_name,
        name: release.name ?? release.tag_name,
        publishedAt: release.published_at,
        assets: assets.map((asset) => ({ id: asset.id, name: asset.name, downloadCount: asset.download_count, size: asset.size })),
      });
    }
    const collectedAt = now.toISOString();
    return {
      version: 1,
      repository,
      collectedAt,
      views: normalizeTrafficDays(views?.views),
      clones: normalizeTrafficDays(clones?.clones),
      snapshot: {
        date: collectedAt.slice(0, 10),
        collectedAt,
        stars: repo.stargazers_count,
        forks: repo.forks_count,
        watchers: repo.subscribers_count,
        // GitHub's open_issues_count includes open pull requests.
        openIssues: repo.open_issues_count,
        trafficWindow: { views: trafficWindow(views), clones: trafficWindow(clones) },
        referrers: referrers.map(({ referrer, count, uniques }) => ({ referrer, count, uniques })),
        paths: paths.map(({ path, title, count, uniques }) => ({ path, title, count, uniques })),
        releases: normalizedReleases,
      },
    };
  };
}

export function cloudflareStore(accountId, namespaceId, token, request = createJsonClient()) {
  const base = `https://api.cloudflare.com/client/v4/accounts/${encodeURIComponent(accountId)}/storage/kv/namespaces/${encodeURIComponent(namespaceId)}/values/`;
  const headers = { Authorization: `Bearer ${token}` };
  return {
    get: (key) => request(base + encodeURIComponent(key), { headers, allowMissing: true }),
    async put(key, value) {
      const result = await request(base + encodeURIComponent(key), {
        method: "PUT",
        headers: { ...headers, "Content-Type": "application/json" },
        body: JSON.stringify(value),
      });
      if (result?.success !== true) throw new Error("Cloudflare KV did not confirm the write");
    },
  };
}

export function memoryStore() {
  const records = new Map();
  return {
    get: async (key) => structuredClone(records.get(key) ?? null),
    put: async (key, value) => { records.set(key, structuredClone(value)); },
  };
}

export function directoryStore(directory) {
  const file = (key) => path.join(directory, `${encodeURIComponent(key)}.json`);
  return {
    async get(key) {
      try {
        return JSON.parse(await readFile(file(key), "utf8"));
      } catch (error) {
        if (error.code === "ENOENT") return null;
        throw error;
      }
    },
    async put(key, value) {
      await mkdir(directory, { recursive: true });
      await writeFile(file(key), `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
    },
  };
}

export async function main(args = process.argv.slice(2), env = process.env) {
  let dryRun = false;
  let output;
  for (let index = 0; index < args.length; index++) {
    if (args[index] === "--dry-run") dryRun = true;
    else if (args[index] === "--output" && args[index + 1] && !args[index + 1].startsWith("--")) {
      output = path.resolve(args[++index]);
      dryRun = true;
    } else throw new Error("Usage: node scripts/insights/collect.mjs [--dry-run] [--output DIRECTORY]");
  }
  const config = validateConfig(JSON.parse(await readFile(CONFIG, "utf8")), { requireNamespace: !dryRun });
  if (!env.GH_INSIGHTS_TOKEN) throw new Error("GH_INSIGHTS_TOKEN is required (Administration: read; Contents: read)");
  if (!dryRun && (!env.CLOUDFLARE_API_TOKEN || !/^[a-f0-9]{32}$/i.test(env.CLOUDFLARE_ACCOUNT_ID ?? ""))) {
    throw new Error("CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID are required for Cloudflare KV");
  }
  const store = output ? directoryStore(output) : dryRun ? memoryStore() : cloudflareStore(env.CLOUDFLARE_ACCOUNT_ID, config.kvNamespaceId, env.CLOUDFLARE_API_TOKEN);
  const { status, summaries } = await collectAll({
    repositories: config.repositories,
    store,
    collectRepository: githubCollector(env.GH_INSIGHTS_TOKEN),
    secrets: [env.GH_INSIGHTS_TOKEN, env.CLOUDFLARE_API_TOKEN],
  });
  console.log(`${dryRun ? "Local preview" : "Cloudflare KV collection"}: ${summaries.length}/${config.repositories.length} repositories saved.`);
  for (const summary of summaries) console.log(`${summary.repository}: ${summary.viewDays} view days, ${summary.cloneDays} clone days, ${summary.snapshotDays} snapshot days.`);
  for (const error of status.errors) console.error(`${error.repository}: ${error.message}`);
  return status.errors.length ? 1 : 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().then((code) => { process.exitCode = code; }).catch((error) => {
    console.error(redactError(error, [process.env.GH_INSIGHTS_TOKEN, process.env.CLOUDFLARE_API_TOKEN]));
    process.exitCode = 1;
  });
}
