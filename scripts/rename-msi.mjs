// Tauri 2.11 强制给 MSI 加 _<language> 后缀（hindsight_0.1.0_x64_en-US.msi），
// 没有内置开关。tauri build 后跑这个脚本把 _en-US 去掉，
// 让本地和 CI 出来的文件名都是 hindsight_<version>_<arch>.msi
import { readdir, rename } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";

const msiDir = "src-tauri/target/release/bundle/msi";

if (!existsSync(msiDir)) {
  // 没生成 MSI（dev / 非 build 命令）—— 跳过
  process.exit(0);
}

const files = await readdir(msiDir);
for (const f of files) {
  if (!f.endsWith(".msi")) continue;
  const m = f.match(/^(.+)_([a-z]{2}-[A-Z]{2})\.msi$/);
  if (!m) continue;
  const newName = `${m[1]}.msi`;
  const from = path.join(msiDir, f);
  const to = path.join(msiDir, newName);
  // 同名文件存在就先删（可能是上次 build 残留）
  if (existsSync(to)) {
    await rename(to, to + ".bak");
  }
  await rename(from, to);
  console.log(`renamed: ${f} -> ${newName}`);
}
