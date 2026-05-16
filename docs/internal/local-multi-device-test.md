# 本机模拟两台设备做云同步测试

跨设备同步的 bug 反复需要"mac + Win 两台真机"才能验证，调试代价大。这份文档讲怎么在**同一台 mac**上跑两个**完全独立**的 Hindsight 实例，让它们像两台真设备一样互相 push/pull、对得上数字 / 对不上 / 触发 tombstone / 复现孤儿 session 等所有跨设备场景。

## 原理

两个 Hindsight 进程，各自有：
- 独立的 data 目录（含 `device.json`、SQLite、screenshots/、icons/、bootstrap.json）
- 独立的 `device_id`（首次启动随机生成 UUID 写进各自的 `device.json`）
- 各自的 window + tray + capture loop + sync engine

但登**同一个 Google 账号**，所以它们 push/pull 同一个 Drive `appDataFolder`。从同步引擎角度，跟两台真机 100% 等价。

## 两个开关

代码里加了两个 env var，仅给这个场景用，**生产路径绝不会设**：

| Env var | 默认 | 设了之后 |
|---|---|---|
| `HINDSIGHT_DATA_DIR=/path` | 走 `bootstrap.json` 或系统默认 | 强制用这个路径当 data root |
| `HINDSIGHT_MULTI_INSTANCE=1` | `tauri_plugin_single_instance` 拦截第二个进程 | 跳过 single instance gate，允许同时跑多个 |

代码位置：
- [src-tauri/src/bootstrap.rs:data_root](../../src-tauri/src/bootstrap.rs)
- [src-tauri/src/lib.rs](../../src-tauri/src/lib.rs)（搜 `HINDSIGHT_MULTI_INSTANCE`）

## 快速操作

### 准备一次 release build

```bash
cd /Users/kyotomogen/Program_Files/Hindsight
npm run tauri build
```

跑完 binary 在 `src-tauri/target/release/bundle/macos/hindsight.app`。也可以 `cargo build --release` 直接拿可执行档（在 `src-tauri/target/release/hindsight`）。

> **为什么不用 `npm run tauri dev`**：dev 模式起 vite 在 5173 端口，第二个实例会撞端口。release build 把前端打成静态文件嵌进 binary，绕开 vite 完全。

### 启动实例 A（模拟"mac"）

终端 1：
```bash
export HINDSIGHT_DATA_DIR=/tmp/hindsight_test_a
export HINDSIGHT_MULTI_INSTANCE=1
export RUST_LOG=hindsight=info
/Users/kyotomogen/Program_Files/Hindsight/src-tauri/target/release/hindsight
```

首次启动会自动：
- 在 `/tmp/hindsight_test_a/` 建数据目录
- 写一个新的 `device.json`（含随机 device_id UUID）
- 跑 schema migrations 到 v26
- 开窗口

进 app 后到「云同步」面板登 Google 账号。

### 启动实例 B（模拟"Win"）

终端 2（同时跑，不要关 A）：
```bash
export HINDSIGHT_DATA_DIR=/tmp/hindsight_test_b
export HINDSIGHT_MULTI_INSTANCE=1
export RUST_LOG=hindsight=info
/Users/kyotomogen/Program_Files/Hindsight/src-tauri/target/release/hindsight
```

同样首次会建 `/tmp/hindsight_test_b/`、新 device_id、跑 migrations。登**同一个**Google 账号。

两个窗口现在都开着，各自有独立的数据 + UI。

### 验证两边能互相看到

1. 两个实例都「云同步」面板登成功（`signedIn = true`）
2. 在「设备」页应当能看到对方（pull 拉到对端 `device.<other_id>.meta.json` 后会显示）
3. 用一会儿（每个实例切几次焦点让 seal_session 触发 → 入 outbox → push）
4. 两端「今日总览」选「所有设备」应当能看到对方的活动行

### 复现你之前的 bug

- **「清空云端」效果**：实例 A 上点「清空云端数据」→ 看实例 B 多久后看不到 A 历史 mirror（tombstone 走完整链路）
- **孤儿 unsealed 行**：人为 kill 掉实例 A 的进程（`pkill -9 hindsight`）后重启，看 `purge_orphan_sessions` 日志输出多少行被清
- **mirror 收敛**：实例 A 清空、push 空 ndjson → 看实例 B pull 后 mac mirror 是否被 DELETE 干净

## 直接查实例 DB

```bash
# 实例 A 的 DB（路径取决于登录的 account uid）
ls /tmp/hindsight_test_a/
sqlite3 /tmp/hindsight_test_a/hindsight.*.sqlite ".tables"

# 看 activities 表 sealed/unsealed 分布
sqlite3 /tmp/hindsight_test_a/hindsight.*.sqlite "
  SELECT
    CASE WHEN duration_secs > 0 THEN 'sealed' ELSE 'unsealed' END AS kind,
    process_name, COUNT(*), SUM(duration_secs)
  FROM activities WHERE local_date='$(date +%Y-%m-%d)'
  GROUP BY kind, process_name
  ORDER BY SUM(duration_secs) DESC"
```

## 清场

```bash
# 关两个 app 进程
pkill -f "hindsight_test_a\|hindsight_test_b" 2>/dev/null
# 或
killall hindsight

# 删测试 data
rm -rf /tmp/hindsight_test_a /tmp/hindsight_test_b

# Drive 上的 device.<a_id>.* / device.<b_id>.* 残留文件需要手动清，或在实例 A/B 各点一次「清空云端数据」（会自动加 tombstone）
```

## 注意

- **OAuth 客户端**：两个实例用同一个 OAuth Client ID。Google 同账号同 client 多 token 是合法的，但有些场景可能有限制；如果遇到 token 互相 revoke 的情况，分两个 Google client 各放一个 `settings.googleClientId`
- **macOS keyring**：两个实例共用同一 macOS keychain；refresh_token 加密存储 key 是 per-app 的，会互相覆盖。**这是已知坑**。两个实例之间频繁切登录可能导致一个被踢出。规避：测试期保持两个都登着别重复登
- **截图目录**：两个实例的 `screenshots/` 完全分开，不会撞
- **AI 模型 / runtime**：单独的 `ai/` 目录，会重复下载，浪费磁盘但功能正常
- **Tauri auto-update**：两个实例的 latest.json 检查会同时跑，没坏处但有点噪音；可以在 `settings.auto_update_enabled = false` 关掉
