// Stands in for @tauri-apps/api/path. The demo pretends to run on Windows (see plugin-os.ts).

/** Returns the demo user's Downloads folder. */
export async function downloadDir(): Promise<string> {
  return "C:\\Users\\demo\\Downloads";
}

/** Joins path parts with the Windows separator. */
export async function join(...paths: string[]): Promise<string> {
  return paths.join("\\");
}
