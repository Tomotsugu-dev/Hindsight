import type { DownloadSources } from "../api/hindsight";

export const DEFAULT_DOWNLOAD_SOURCES: DownloadSources = {
  mode: "official",
  huggingfaceUrl: "https://hf-mirror.com",
  githubProxyUrl: "https://gh-proxy.org",
  nugetUrl: "https://mirrors.huaweicloud.com/artifactory/api/nuget/v3/nuget-remote",
};
