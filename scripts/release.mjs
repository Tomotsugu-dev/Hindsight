// 一键发版：本地 build + 签名 + 上传到 GitHub Release，跳过 CI 编译。
//
// 用法：
//   1. 改 package.json / Cargo.toml / tauri.conf.json 的 version 为目标版本号
//   2. git commit -m "Bump version to X.Y.Z" && git push
//   3. npm run release
//
// 前置条件：
//   - 私钥在 ~/.tauri/hindsight_updater.key （首次跑 npx tauri signer generate 生成）
//   - 已装 GitHub CLI（gh）并登录（gh auth login）
//   - 当前分支是 main，working tree 干净
//
// 脚本会：
//   - 校验干净的 main 分支
//   - 用环境变量 TAURI_SIGNING_PRIVATE_KEY 跑 npm run tauri build
//   - 生成 latest.json（updater 用的 manifest）
//   - 推 v<version> tag
//   - gh release create 上传 setup.exe + .sig + latest.json
import { execSync, spawnSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";
import os from "node:os";

const REPO = "Tomotsugu-dev/Hindsight";
const KEY_FILE = path.join(os.homedir(), ".tauri", "hindsight_updater.key");

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

async function main() {
  // —— 1. 环境检查 ——
  if (!existsSync(KEY_FILE))
    fail(`Signing key missing: ${KEY_FILE}\n  生成：npx @tauri-apps/cli signer generate -w ${KEY_FILE}`);

  const status = capture("git status --porcelain").trim();
  if (status) fail(`Working tree not clean:\n${status}`);

  const branch = capture("git branch --show-current").trim();
  if (branch !== "main") fail(`Not on main (current: ${branch})`);

  // —— 2. 读版本号 ——
  const pkg = JSON.parse(await readFile("package.json", "utf-8"));
  const version = pkg.version;
  const tag = `v${version}`;
  console.log(`\n=== Releasing ${tag} ===`);

  // —— 3. tag 不能已存在 ——
  const localTag = spawnSync("git", ["rev-parse", "--verify", tag], {
    stdio: "ignore",
  });
  if (localTag.status === 0)
    fail(`Tag ${tag} already exists locally. Bump version (package.json + Cargo.toml + tauri.conf.json) first.`);

  // —— 4. push main 最新 ——
  run("git push origin main");

  // —— 5. 加载私钥 ——
  const privateKey = (await readFile(KEY_FILE, "utf-8")).trim();

  // —— 6. 本地 build ——
  // 把签名密钥通过 env 传给 tauri build；TAURI_SIGNING_PRIVATE_KEY_PASSWORD 留空（密钥无密码）
  run("npm run tauri build", {
    env: {
      ...process.env,
      TAURI_SIGNING_PRIVATE_KEY: privateKey,
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "",
    },
  });

  // —— 7. 检查产物 ——
  const bundleDir = "src-tauri/target/release/bundle/nsis";
  const exeName = `hindsight_${version}_x64-setup.exe`;
  const sigName = `${exeName}.sig`;
  const exePath = path.join(bundleDir, exeName);
  const sigPath = path.join(bundleDir, sigName);

  if (!existsSync(exePath)) fail(`Missing: ${exePath}`);
  if (!existsSync(sigPath))
    fail(`Missing signature: ${sigPath}\n  TAURI_SIGNING_PRIVATE_KEY 没生效；检查 ~/.tauri/hindsight_updater.key 是否完整`);

  // —— 8. 生成 latest.json（Tauri Updater 的 manifest）——
  // macOS 占位：当前没付 Apple Developer，没法签名 .app.tar.gz 让 updater 真做静默替换。
  // 但仍要列在 platforms 里，否则 macOS 端 check() 找不到对应平台 key 会返回 null
  // （= "已是最新版"），用户永远收不到新版通知。
  // - url 指向 release tag 页面（前端 useUpdater 在 macOS 分支调 openUrl 跳浏览器）
  // - signature 复用 Windows 的真实签名当 dummy；check() 阶段 Tauri 不验证 signature
  //   内容，只在 downloadAndInstall 时才用——而 macOS 端我们根本不调 downloadAndInstall
  const sig = (await readFile(sigPath, "utf-8")).trim();
  const tagPageUrl = `https://github.com/${REPO}/releases/tag/${tag}`;
  const latestJson = {
    version,
    notes: `Hindsight ${tag}`,
    pub_date: new Date().toISOString(),
    platforms: {
      "windows-x86_64": {
        signature: sig,
        url: `https://github.com/${REPO}/releases/download/${tag}/${exeName}`,
      },
      "darwin-x86_64": {
        signature: sig,
        url: tagPageUrl,
      },
      "darwin-aarch64": {
        signature: sig,
        url: tagPageUrl,
      },
    },
  };
  const latestPath = "latest.json";
  await writeFile(latestPath, JSON.stringify(latestJson, null, 2));
  console.log(`Generated ${latestPath}`);

  // —— 9. tag + push ——
  run(`git tag ${tag}`);
  run(`git push origin ${tag}`);

  // —— 10. 创建 release + 上传 ——
  // --generate-notes 让 GitHub 自动从 commits 生成 release notes
  // 用 cmd.exe 友好的引号格式（脚本主要在 Windows 跑）
  const ghCmd = [
    `gh release create ${tag}`,
    `"${exePath}"`,
    `"${sigPath}"`,
    `${latestPath}`,
    `--title "Hindsight ${tag}"`,
    `--generate-notes`,
  ].join(" ");
  run(ghCmd);

  console.log(`\n✓ Released ${tag}`);
  console.log(`  https://github.com/${REPO}/releases/tag/${tag}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
