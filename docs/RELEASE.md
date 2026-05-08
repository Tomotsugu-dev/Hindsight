# 发版流程

## 一次性配置：GitHub Secrets

`.github/workflows/release.yml` 在 CI 上 build + 签名 + 上传，需要从你本地导出凭证到 GitHub Repository Secrets。一次性做完，之后 `npm run release` 一键发版。

去 [Repo Settings → Secrets and variables → Actions](https://github.com/Tomotsugu-dev/Hindsight/settings/secrets/actions)依次添加这 8 个 secret：

### Tauri Updater（同时签 macOS / Windows 包）

| Secret 名 | 怎么拿 |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | `cat ~/.tauri/hindsight_updater.key`，**整段贴进去**（含 BEGIN/END 行） |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 你生成 key 时的密码；**没设就留空字符串 `""`** |

### Apple 代码签名 + Notarization

| Secret 名 | 怎么拿 |
|---|---|
| `APPLE_CERTIFICATE` | 见下方 "导出 Apple 证书" |
| `APPLE_CERTIFICATE_PASSWORD` | 你导出 .p12 时设的密码 |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Youyan Xu (HJ2YT95D35)` |
| `APPLE_ID` | `heroburnning@gmail.com` |
| `APPLE_PASSWORD` | App-Specific Password（[appleid.apple.com](https://appleid.apple.com) → 安全 → 应用专用密码 → 生成）；本地也存了一份在 `.env.local` |
| `APPLE_TEAM_ID` | `HJ2YT95D35` |

### 导出 Apple 证书 → Base64

```bash
# 1) 在 Keychain Access.app 里：
#    登录 → 我的证书 → 右键 "Developer ID Application: Youyan Xu (HJ2YT95D35)"
#    → 导出 → 格式选 "Personal Information Exchange (.p12)"
#    → 设一个密码（记下来当 APPLE_CERTIFICATE_PASSWORD）→ 存到 ~/Desktop/certificate.p12
#
# 2) 转成单行 base64：
base64 -i ~/Desktop/certificate.p12 | pbcopy
#    粘进 GitHub Secret APPLE_CERTIFICATE
#
# 3) 删本地 .p12（base64 已在 GitHub 了，本地副本是冗余风险）：
rm ~/Desktop/certificate.p12
```

---

## 每次发版

1. 改三处 `version` 字段：
   - `package.json`
   - `src-tauri/Cargo.toml`
   - `src-tauri/tauri.conf.json`

2. 写 release notes：`docs/release-notes/v<version>.md`
   （CI 会读这个文件填进 GitHub Release body 和应用内"检查更新"对话框）

3. 提交 + 推：
   ```bash
   git commit -am "Bump version to X.Y.Z"
   git push
   ```

4. 触发发布：
   ```bash
   npm run release
   ```
   脚本会校验三处版本号一致 + release notes 存在 + push `v<version>` tag。
   tag 推上去后 GitHub Actions 自动跑 build → 6-10 分钟出双平台 release。

5. 看 [Actions 页](https://github.com/Tomotsugu-dev/Hindsight/actions) 等绿勾。

---

## 出错了怎么办

### CI 失败（间歇性，比如 Apple notarize 502）
去 Actions 页对应 run 点 "Re-run all jobs"，同一 tag 复用产物继续。

### CI 失败（配置或代码问题，需要改代码再发）
1. **删 tag**（本地 + 远程）：
   ```bash
   git tag -d vX.Y.Z
   git push origin :refs/tags/vX.Y.Z
   ```
2. 删 GitHub Release（如果已部分创建）：[Releases 页](https://github.com/Tomotsugu-dev/Hindsight/releases) 手动删
3. 修代码 → commit → 重跑 `npm run release`

### 急需发版但 CI 跪了
最后的兜底：本地手动两机编译 + 用 `gh release create` 上传。但这是流程的失败模式，平时别走。

---

## 改 secret 后忘了的细节

- App-Specific Password 一旦泄露，去 [appleid.apple.com](https://appleid.apple.com) revoke + 生成新的，更新 `APPLE_PASSWORD` secret 和 `.env.local` 即可
- Apple Developer ID 证书每年续期一次（看 Keychain 里有效期）；续期后重新导出 .p12 + 更新 `APPLE_CERTIFICATE` secret
- Tauri minisign key **永远不要换**——一旦换，所有现存安装包的自动更新链路就断了（旧 app 只信旧 pubkey）
