# Hindsight 官网

`hindsight.kyosweb.com` 的全部内容：各语言 landing page、可交互的 web demo（`/demo/`）和功能文档站（`/docs/`）。
整站都是静态文件，发布在 Cloudflare Workers 静态资源上，从全球边缘节点直接响应，没有源站。

## 文件

```
web/
├── index.html         简体中文主页（默认语言）
├── en/ ja/ pt/ tw/    其它语言主页
├── site.css           各页共用的配色变量、顶栏和页脚（首页、文档站、主题页、404 都先加载它）
├── home.css           首页样式
├── linux.svg          Linux 下载按钮的图标，取自 Simple Icons 13.21.0（CC0），home.css 用 mask 引用
├── 404.html           找不到页面时返回
├── _headers           响应头（缓存）规则
├── wrangler.jsonc     Cloudflare 部署配置
├── worker.js          只处理 /assets/video/*：给视频补上 Range（206）支持，Safari 播放需要
├── demo-src/          web demo 源码（mock + 入口），复用主应用 src/ 的组件
├── vite.config.ts     web demo 的构建配置
├── themes-src/        主题页 /themes/ 源码，构建配置是自己的 vite.config.ts
├── demo/              ← npm run build:demo 产物，不入仓
├── themes/            ← npm run build:themes 产物，不入仓
└── docs/              ← npm run build:docs 产物，不入仓
```

视频和封面不在 `web/` 里，在 `docs/intro/<lang>/shot_video/`，构建时按页面引用拷到 `/assets/video/`。

## 构建与本地预览

```bash
npm run build:web     # demo + 文档站 + 拼装，产物在 dist-web/
npx wrangler dev --config web/wrangler.jsonc   # 按线上规则（_headers、404）预览
```

`build:web` 会校验所有站内 `src` / `href` / `poster` 都指向存在的文件，有一个缺失就构建失败。

## 资源

- **视频**：`docs/intro/<lang>/shot_video/Hindsight_<lang>.mp4`，单个文件不能超过 25 MiB。
- **视频封面**：同目录的同名 `.jpg`，是视频第一帧。重录视频后重新生成：
  ```bash
  for v in docs/intro/*/shot_video/*.mp4; do ffmpeg -v error -y -i "$v" -frames:v 1 -q:v 4 "${v%.mp4}.jpg"; done
  ```
- **首页功能卡片**：`web/assets/home/<名字>_<lang>.mp4` 和同名 `.webp`（第一帧，当封面）。960×720，30 fps，H.264，无音轨，单个不超过 400 KB。卡片只有约 340 CSS px 宽，所以画面里的字至少 31 px 高。
- **logo** 通过 [jsDelivr](https://www.jsdelivr.com/) 引用主分支文件 `https://cdn.jsdelivr.net/gh/Tomotsugu-dev/Hindsight@main/...`。
- **下载按钮** 默认指向 `releases/latest`，JS 加载后改写为具体安装包直链，并显示文件大小。

## 部署

`.github/workflows/web.yml` 自动完成：

- push 到 `main`：构建并发布到 `hindsight.kyosweb.com`，约 1–2 分钟生效。
- 本仓库分支的 PR：上传预览版本，地址是 `<分支名>-hindsight.<账号子域>.workers.dev`，不影响线上。

需要的仓库 secrets（Settings → Secrets and variables → Actions）：

| 名称 | 值 |
|---|---|
| `CLOUDFLARE_API_TOKEN` | Cloudflare 控制台 → My Profile → API Tokens → Create Token → 模板 **Edit Cloudflare Workers** |
| `CLOUDFLARE_ACCOUNT_ID` | `npx wrangler whoami` 显示的 Account ID |

手动发布（wrangler 需要 Node 22 以上，并先 `npx wrangler login`）：

```bash
npm run build:web
npx wrangler deploy --config web/wrangler.jsonc
```

回滚：Cloudflare 控制台 → Workers & Pages → hindsight → Deployments，选旧版本 Rollback；或 `npx wrangler rollback --config web/wrangler.jsonc`。

## 修改清单

- 全局配色 token、顶栏、页脚：`site.css`，改一处全站生效
- 首页其余样式：`home.css`；文档站样式：`scripts/docs/docs.css`
- 各 section 文案：各语言 `index.html` 里有注释 `<!-- ============ XXX ============ -->` 分隔
- 视频：替换 `docs/intro/<lang>/shot_video/` 下的 mp4，并重新生成封面
- 缓存时长：`_headers`

## 后续可加

- Pro waitlist 邮件订阅区块（Tally embed）
- Analytics（Cloudflare Web Analytics 免费、隐私友好）
