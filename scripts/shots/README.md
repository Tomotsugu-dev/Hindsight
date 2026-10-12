# 文档截图

两个脚本都使用网页 demo 自带的数据和真实页面组件截图，构建 demo、打开页面、截图这些步骤共用 `web-demo.mjs`：

- `features.mjs`：给 `docs/features/zh` 下的功能文档截图。
- `readme/shoot.mjs`：给 README 截介绍图，见最后一节。

窗口尺寸来自 `src-tauri/tauri.macos.conf.json`，带 macOS 红绿灯；以 2 倍像素保存，默认输出 1920 × 1440 的整窗口图片。

在仓库根目录运行。需要已安装项目依赖，以及 Google Chrome 或 Playwright Chromium。

## 屏幕记忆与搜索

```sh
npm run shots:features -- --only search-results,search-preview
```

脚本自动构建 demo、启动本地预览、打开浏览器，进入「全部历史 → 搜索」并搜索 `OCR`。等待结果和缩略图加载完成后保存搜索页，再打开「Optical character recognition — Wikipedia」结果保存截图预览。完成后关闭浏览器和预览服务。

生成的图片：

- `docs/features/zh/imgs/screen-memory-search/search-results.png`
- `docs/features/zh/imgs/screen-memory-search/search-preview.png`

文档在 `docs/features/zh/screen-memory-search.md` 中引用这两张图。重跑命令会更新对应图片。

## 其他截图与参数

```sh
# 查看支持的截图名称和选项
npm run shots:features -- --help

# 生成全部功能截图
npm run shots:features

# 使用英文界面，保存到英文文档的图片目录
npm run shots:features -- --only search-results,search-preview --lang en --out docs/features/en/imgs
```

| 参数 | 用途 | 默认值 |
| --- | --- | --- |
| `--only` | 要生成的截图名称，以逗号分隔 | 全部截图 |
| `--out` | 输出根目录，脚本按文档名创建子目录 | `docs/features/zh/imgs` |
| `--lang` | 界面语言 | `zh-CN` |
| `--theme` | `minimal`、`default` 或 `dark` | `minimal` |
| `--date` | 演示数据的日期，格式为 `YYYY-MM-DD` | 东京当天 |

截图配置集中在 `features.mjs` 的 `SHOTS` 数组中。搜索截图的 `query` 指定关键词，`resultTitle` 指定结果标题，`preview: true` 表示打开该结果的截图预览，`highlights: true` 表示等待高亮框出现后再截图。

## README 介绍图

```sh
# 一次生成所有语言 README 用的图（6 种语言 × 8 张），保存到 docs/intro/<语言目录>/imgs
npm run shots:readme

# 只生成某几种语言
npm run shots:readme -- --lang ja,en

# 只重新生成其中几张
npm run shots:readme -- --only daily,cloud_sync
```

文件名就是 README 里引用的名字：`daily`、`app_detail`、`weekly`、`monthly`、`monthly_cal`、`ai_summary`、`ai_chatbot`、`cloud_sync`。`cloud_sync` 会像用户一样填一组假凭证登录 demo，拍的是登录后的样子。

| 参数 | 用途 | 默认值 |
| --- | --- | --- |
| `--only` | 要生成的截图名称，以逗号分隔 | 全部截图 |
| `--lang` | 界面语言，以逗号分隔 | `zh-CN,zh-TW,en,ja,pt-BR,es` |
| `--out` | 输出根目录，图保存到 `<语言目录>/imgs` | `docs/intro` |
| `--theme` | `minimal`、`default` 或 `dark` | `minimal` |
| `--date` | 演示数据的日期，格式为 `YYYY-MM-DD` | 东京最近一个周日 |

语言对应的目录：`zh-CN` → `zh`，`zh-TW` → `zh-TW`，`en` → `en`，`ja` → `ja`，`pt-BR` → `pt`，`es` → `es`。

默认日期取周日，是因为演示数据里今天以后没有记录，周统计要这天才七天都满；月统计在月初只会有前几天的柱子。
