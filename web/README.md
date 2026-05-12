# Hindsight Landing Page

`hindsight.kyosweb.com` 的静态 landing page。纯 HTML + CSS + 少量 JS，零构建步骤。

## 文件

```
web/
├── index.html       中文版主页
├── styles.css       全部样式
└── README.md        本文件
```

## 本地预览

任选一种：

```bash
# 方式 1: Python（最简单）
cd web && python -m http.server 8000
# 浏览器开 http://localhost:8000

# 方式 2: Node
npx serve web

# 方式 3: VS Code Live Server 插件，右键 index.html → Open with Live Server
```

> ⚠️ **直接 file:// 打开 index.html 视频可能不播**——视频用了 GitHub user-attachments URL，
> 现代浏览器对 file:// scheme 限制 cors / 媒体加载。一定用本地 server 预览。

## 资源策略

- **图片**（logo + 截图）通过 [jsDelivr CDN](https://www.jsdelivr.com/) 引用 GitHub 主分支文件：
  ```
  https://cdn.jsdelivr.net/gh/Tomotsugu-dev/Hindsight@main/...
  ```
  好处：自动跟主分支同步；CDN 全球加速；不用复制图片到 web/。
- **视频** 使用 GitHub user-attachments CDN URL（README.zh.md 同款）
- **下载按钮** 默认指向 `releases/latest`，JS 加载后改写为具体安装包直链 + 显示版本和大小

## 部署 Cloudflare Pages

### Step 1: Cloudflare Pages 后台

1. 进 [dash.cloudflare.com](https://dash.cloudflare.com) → Workers & Pages → Pages
2. **Connect to Git** → 选 Hindsight repo
3. 部署设置：
   - **Production branch**: `main`
   - **Build command**: 留空（纯静态）
   - **Build output directory**: `web`
   - **Root directory**: 留空（用 repo 根）
4. **Save and Deploy**

部署后会拿到 `<project-name>.pages.dev` 默认域名。

### Step 2: 绑定自定义域名

1. Pages 项目 → **Custom domains** → **Set up a custom domain**
2. 输入 `hindsight.kyosweb.com`
3. Cloudflare 自动添加 CNAME 记录指向 `<project-name>.pages.dev`
   - 你 kyosweb.com 在 Cloudflare DNS 下 → 自动完成
   - 不在 Cloudflare → 手动到 DNS 提供商加 CNAME

5 分钟后 `hindsight.kyosweb.com` 生效，自动 HTTPS。

### 后续 push 自动部署

之后每次 `git push origin main`，Cloudflare Pages 自动检测 `web/` 改动并重新部署，~30 秒生效。

## 多语言扩展（未来）

加 en / ja 版本时：

```
web/
├── index.html       中文（默认）
├── en/index.html    英文
└── ja/index.html    日文
```

Nav 里的 lang switch 改成实际链接 `/en/` / `/ja/`。共用 `styles.css`。

## 修改清单

改文案 / 配色 / 布局：

- 全局配色 token：`styles.css` 顶部 `:root { ... }`
- 各 section 文案：`index.html` 里有注释 `<!-- ============ XXX ============ -->` 分隔
- 视频 URL：搜 `user-attachments/assets/df92b5b8` 替换
- 截图：搜 `intro_zh/imgs/` 替换文件名

## 后续可加

- 多语言版本（en, ja）
- Pro waitlist 邮件订阅区块（Tally embed）
- Analytics（Cloudflare Web Analytics 免费、隐私友好）
- 博客 / 文档子站（`docs/` / `blog/` 子目录）
