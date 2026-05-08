// 一键发版（CI 路径）：本地只做"校验 → push tag"，
// 真正的 build / sign / notarize / 上传 由 .github/workflows/release.yml 在
// GitHub Actions 上跑（macOS + Windows 双平台并行）。
//
// 用法：
//   1. 改 package.json / Cargo.toml / tauri.conf.json 的 version 为目标版本号
//   2. 写 docs/release-notes/v<version>.md（应用内"检查更新"对话框 + GitHub Release body 都用它）
//   3. git commit -m "Bump version to X.Y.Z" && git push
//   4. npm run release
//
// 脚本会：
//   - 校验干净的 main 分支
//   - 校验三处 version 一致
//   - 校验 release notes 文件存在
//   - tag 不能已存在
//   - push v<version> tag → 触发 .github/workflows/release.yml
//
// 如果 CI 失败（比如 Apple notarize 间歇 502），可以在 Actions 页"Re-run all jobs"
// 重跑同一 tag。

import { execSync, spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";

const REPO = "Tomotsugu-dev/Hindsight";

function run(cmd, opts = {}) {
  console.log(`\n> ${cmd}`);
  execSync(cmd, { stdio: "inherit", shell: true, ...opts });
}

function capture(cmd) {
  return execSync(cmd, { shell: true }).toString();
}

function fail(msg) {
  console.error(`\n✗ ${msg}`);
  process.exit(1);
}

async function readVersionFromPkg(file) {
  const content = await readFile(file, "utf-8");
  if (file.endsWith(".json")) {
    return JSON.parse(content).version;
  }
  // Cargo.toml: 抠 [package] 段下第一个 version = "..."
  const m = content.match(/^\s*\[package\][^\[]*?^\s*version\s*=\s*"([^"]+)"/ms);
  return m?.[1];
}

async function main() {
  // —— 1. 干净 main 分支 ——
  const status = capture("git status --porcelain").trim();
  if (status) fail(`Working tree not clean:\n${status}`);

  const branch = capture("git branch --show-current").trim();
  if (branch !== "main") fail(`Not on main (current: ${branch})`);

  // —— 2. 三处版本号一致 ——
  const pkgVer = await readVersionFromPkg("package.json");
  const cargoVer = await readVersionFromPkg("src-tauri/Cargo.toml");
  const tauriVer = await readVersionFromPkg("src-tauri/tauri.conf.json");

  if (pkgVer !== cargoVer || pkgVer !== tauriVer) {
    fail(
      `Version mismatch:\n  package.json = ${pkgVer}\n  Cargo.toml   = ${cargoVer}\n  tauri.conf.json = ${tauriVer}`,
    );
  }
  const version = pkgVer;
  const tag = `v${version}`;
  console.log(`\n=== Releasing ${tag} ===`);

  // —— 3. release notes 必须存在 ——
  const notesPath = path.join("docs", "release-notes", `${tag}.md`);
  if (!existsSync(notesPath)) {
    fail(
      `Missing ${notesPath}\n  CI 会读这个文件填进 GitHub Release body 和应用内"检查更新"对话框，\n  没有就发了等于用户看不到 changelog。先写完再 release。`,
    );
  }

  // —— 4. tag 不能已存在 ——
  const localTag = spawnSync("git", ["rev-parse", "--verify", tag], {
    stdio: "ignore",
  });
  if (localTag.status === 0) {
    fail(`Tag ${tag} already exists locally. Bump version first.`);
  }

  // —— 5. push 最新 main ——
  run("git push origin main");

  // —— 6. 创建并 push tag → 触发 CI ——
  run(`git tag ${tag}`);
  run(`git push origin ${tag}`);

  console.log(`\n✓ Pushed ${tag}. CI 已开始构建：`);
  console.log(`  https://github.com/${REPO}/actions`);
  console.log(`\n  ~6-10 分钟后产物会出现在：`);
  console.log(`  https://github.com/${REPO}/releases/tag/${tag}`);
  console.log(`\n  如果 CI 失败，去 Actions 页 "Re-run all jobs" 重试同一 tag。`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
