// Mock 替换 @tauri-apps/api/app

import { version } from "../../../package.json";

/** Returns the version in package.json, so the About tab matches the latest release. */
export async function getVersion(): Promise<string> {
  return version;
}

export async function getName(): Promise<string> {
  return "Hindsight";
}

export async function getTauriVersion(): Promise<string> {
  return "2.0.0";
}
