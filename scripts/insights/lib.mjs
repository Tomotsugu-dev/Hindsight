const REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const DATE = /^\d{4}-\d{2}-\d{2}$/;

export function validateConfig(config, { requireNamespace = true } = {}) {
  if (config?.version !== 1 || !Array.isArray(config.repositories) || !config.repositories.length) {
    throw new Error("Insights config must have version 1 and at least one repository");
  }
  if (config.repositories.some((repository) => !REPOSITORY.test(repository))) {
    throw new Error("Insights repositories must use owner/repository names");
  }
  if (new Set(config.repositories.map((repository) => repository.toLowerCase())).size !== config.repositories.length) {
    throw new Error("Insights config contains duplicate repositories");
  }
  if (requireNamespace && !/^[a-f0-9]{32}$/i.test(config.kvNamespaceId ?? "")) {
    throw new Error("Set kvNamespaceId in scripts/insights/config.json before collecting to Cloudflare");
  }
  return config;
}

function count(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`Invalid ${label} count from GitHub`);
  return value;
}

export function normalizeTrafficDays(rows) {
  if (!Array.isArray(rows)) throw new Error("Missing daily traffic records from GitHub");
  return rows.map((row) => {
    if (typeof row.timestamp !== "string" || !/^\d{4}-\d{2}-\d{2}T00:00:00Z$/.test(row.timestamp)) {
      throw new Error("GitHub traffic timestamp is not a UTC calendar day");
    }
    return { date: row.timestamp.slice(0, 10), count: count(row.count, "traffic"), uniques: count(row.uniques, "unique traffic") };
  });
}

export function trafficWindow(data) {
  return { count: count(data?.count, "traffic window"), uniques: count(data?.uniques, "unique traffic window") };
}

// A missing day is unknown, not zero. New API values replace matching dates,
// including a valid zero, while dates outside the current window remain intact.
export function mergeByDate(previous, incoming) {
  const byDate = new Map();
  for (const row of [...previous, ...incoming]) {
    if (!row || !DATE.test(row.date)) throw new Error("Invalid date in Insights history");
    byDate.set(row.date, row);
  }
  return [...byDate.values()].sort((a, b) => a.date.localeCompare(b.date));
}

export function mergeHistory(previous, collected) {
  if (previous && (previous.version !== 1 || previous.repository !== collected.repository)) {
    throw new Error("Stored Insights history has an incompatible version or repository");
  }
  return {
    version: 1,
    repository: collected.repository,
    firstCollectedAt: previous?.firstCollectedAt ?? collected.collectedAt,
    lastCollectedAt: collected.collectedAt,
    views: mergeByDate(previous?.views ?? [], collected.views),
    clones: mergeByDate(previous?.clones ?? [], collected.clones),
    snapshots: mergeByDate(previous?.snapshots ?? [], [collected.snapshot]),
  };
}

export function historyKey(repository) {
  return `history:v1:${repository}`;
}

export function snapshotKey(repository, date) {
  return `snapshot:v1:${repository}:${date}`;
}

export function redactError(error, secrets = []) {
  let message = error instanceof Error ? error.message : String(error);
  for (const secret of secrets.filter(Boolean)) message = message.replaceAll(secret, "[redacted]");
  return message.slice(0, 500);
}

// Only mark a repository successful after both its backup and merged history
// have been saved. A failed request must never turn yesterday's data into zero.
export async function collectAll({ repositories, store, collectRepository, now = new Date(), secrets = [] }) {
  const lastAttemptAt = now.toISOString();
  const previous = await store.get("status:v1");
  if (previous && (previous.version !== 1 || !Array.isArray(previous.repositories))) {
    throw new Error("Stored Insights status has an incompatible schema");
  }
  const repositoryStatus = new Map((previous?.repositories ?? []).map((row) => [row.repository, row]));
  const errors = [];
  const summaries = [];
  for (const repository of repositories) {
    try {
      const history = await store.get(historyKey(repository));
      const collected = await collectRepository(repository, now);
      const merged = mergeHistory(history, collected);
      await store.put(snapshotKey(repository, collected.snapshot.date), collected);
      await store.put(historyKey(repository), merged);
      repositoryStatus.set(repository, {
        repository,
        firstCollectedAt: merged.firstCollectedAt,
        lastSuccessAt: collected.collectedAt,
      });
      summaries.push({ repository, viewDays: merged.views.length, cloneDays: merged.clones.length, snapshotDays: merged.snapshots.length });
    } catch (error) {
      errors.push({ repository, message: redactError(error, secrets) });
    }
  }
  const status = {
    version: 1,
    lastAttemptAt,
    lastSuccessAt: errors.length ? (previous?.lastSuccessAt ?? null) : lastAttemptAt,
    repositories: repositories.map((repository) => repositoryStatus.get(repository) ?? {
      repository,
      firstCollectedAt: null,
      lastSuccessAt: null,
    }),
    errors,
  };
  await store.put("status:v1", status);
  return { status, summaries };
}

export function createJsonClient({ fetchImpl = fetch, sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)), attempts = 4 } = {}) {
  return async function request(url, { headers = {}, method = "GET", body, allowMissing = false } = {}) {
    for (let attempt = 0; attempt < attempts; attempt++) {
      let response;
      try {
        response = await fetchImpl(url, { method, headers, body, signal: AbortSignal.timeout(30_000) });
      } catch (error) {
        if (attempt + 1 === attempts) throw new Error(`Network request failed for ${new URL(url).pathname}: ${error.name ?? "error"}`);
        await sleep(1000 * 2 ** attempt);
        continue;
      }
      const rateLimited = response.status === 429 || (response.status === 403 && response.headers.get("x-ratelimit-remaining") === "0");
      if ((rateLimited || response.status >= 500) && attempt + 1 < attempts) {
        const retryAfter = Number(response.headers.get("retry-after"));
        const delay = Number.isFinite(retryAfter) && retryAfter > 0 ? Math.min(retryAfter * 1000, 60_000) : 1000 * 2 ** attempt;
        await response.body?.cancel();
        await sleep(delay);
        continue;
      }
      if (allowMissing && response.status === 404) return null;
      if (!response.ok) {
        // API response bodies are deliberately omitted from logs. They can
        // contain request details, and HTTP status/path are sufficient here.
        await response.body?.cancel();
        throw new Error(`${method} ${new URL(url).pathname} returned HTTP ${response.status}`);
      }
      if (response.status === 204) return null;
      const text = await response.text();
      if (!text) return null;
      try {
        return JSON.parse(text);
      } catch {
        throw new Error(`${method} ${new URL(url).pathname} did not return JSON`);
      }
    }
    throw new Error("Request retry limit reached");
  };
}
