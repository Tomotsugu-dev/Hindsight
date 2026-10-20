// Plugin commands and their data. Types mirror src-tauri/src/plugins/ and repo/plugins.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Where a plugin page opens. `null`: not in the sidebar, opened only from the settings. */
export type OpensIn = "main" | "window" | null;

/** How a version relates to the installed one. */
export type InstallKind = "new" | "update" | "downgrade" | "same";

export interface UiSettings {
  hasPage: boolean;
  opensIn: OpensIn;
  windowX: number | null;
  windowY: number | null;
  windowWidth: number;
  windowHeight: number;
}

/** One row of the plugins table. */
export interface InstalledPlugin {
  id: string;
  name: string;
  version: string;
  enabled: boolean;
  /** Permissions the user approved */
  permissions: string[];
  storageMb: number | null;
  ui: UiSettings;
  runsInBackground: boolean;
  installedAt: string;
}

/** One version in the online list. Its fields keep the plugins.json names. */
export interface OnlineVersion {
  version: string;
  url: string;
  sha256: string;
  permissions: string[];
  storage_mb: number | null;
  kind: InstallKind;
}

export interface OnlinePlugin {
  id: string;
  name: string;
  author: string;
  description: string;
  repo: string;
  /** Newest first */
  versions: OnlineVersion[];
  /** The icon in the official repository. It may not exist; show the default icon then */
  icon_url: string | null;
  /** Made by Hindsight: `repo` is under the official account */
  official: boolean;
}

export interface Manifest {
  id: string;
  name: string;
  version: string;
  permissions: string[];
  storageMb: number | null;
  opensIn: OpensIn;
  window: { width: number; height: number } | null;
  runsInBackground: boolean;
}

/** A zip in the plugin zip folder. `error` is set when it cannot be installed. */
export interface LocalZip {
  fileName: string;
  manifest: Manifest | null;
  kind: InstallKind | null;
  error: string | null;
}

export interface Market {
  installed: InstalledPlugin[];
  /** Online entries of installed plugins, for updates */
  installedOnline: OnlinePlugin[];
  downloadable: OnlinePlugin[];
  localZips: LocalZip[];
  /** Set when the online list could not be fetched; the rest is still filled */
  onlineError: string | null;
}

/** An enabled plugin that has a page. */
export interface PluginPage {
  id: string;
  name: string;
  opensIn: OpensIn;
  url: string;
}

export const pluginsApi = {
  /** Fetches the online list from GitHub, so call it only when the user opens the market. */
  openMarket: () => invoke<Market>("plugin_open_market"),
  install: (fileName: string, approved: string[], storageMb: number | null) =>
    invoke<void>("plugin_install", { fileName, approved, storageMb }),
  installOnline: (id: string, version: string, approved: string[], storageMb: number | null) =>
    invoke<void>("plugin_install_online", { id, version, approved, storageMb }),
  uninstall: (id: string, deleteData: boolean) =>
    invoke<void>("plugin_uninstall", { id, deleteData }),
  setEnabled: (id: string, enabled: boolean) => invoke<void>("plugin_set_enabled", { id, enabled }),
  openZipsFolder: () => invoke<void>("plugin_open_zips_folder"),
  pages: () => invoke<PluginPage[]>("plugin_pages"),
  /** The path of icon.png of each installed plugin that has one, by plugin id */
  icons: () => invoke<Record<string, string>>("plugin_icons"),
  setOpensIn: (id: string, opensIn: OpensIn) =>
    invoke<void>("plugin_set_opens_in", { id, opensIn }),
  openWindow: (id: string) => invoke<void>("plugin_open_window", { id }),
};

/** Calls `listener` with the new page list whenever a plugin changes. */
export function onPagesChanged(listener: (pages: PluginPage[]) => void): Promise<UnlistenFn> {
  return listen<PluginPage[]>("plugin-pages-changed", (event) => listener(event.payload));
}
