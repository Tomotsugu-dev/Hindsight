#!/usr/bin/env bash
# macos-mem-watch.sh — 给 macOS 上的 Hindsight **整个进程组**长期采样 RSS / VSZ，
# 包括主 Rust 进程 + 3 个 WKWebView XPC helpers（WebContent / GPU / Networking）。
#
# 已知：主泄漏发生在 WebView 进程（com.apple.WebKit.WebContent）；主进程的
# autoreleasepool 修复对它无效。本脚本按 role 分别打印斜率，重点关注 webview。
#
# 关键点（看代码前先理解）：
#   - WKWebView helper 是系统 XPC 服务，binary 路径里完全不含 "hindsight"。
#     无法用名字匹配。本脚本用"启动时间（etimes，秒）与主进程相近"做归属——
#     Tauri 启动时几乎同时拉起 3 个 helper，etimes 差 ≤ ETIME_TOL 秒就归到这个 app。
#   - WebContent 在 crash 后会被 WebKit 自动重生，新 etimes 不再匹配。脚本每 5 tick
#     重发现 PIDs，但若新 helper 起得晚很多，可能漏。当前阈值 ETIME_TOL=60s 平衡
#     "鲁棒识别 vs 误抓其它 app helper"。
#   - macOS 默认 bash 是 3.2，没有 `declare -A`，所有 per-role 聚合在 awk 里做。
#
# 用法：
#   scripts/macos-mem-watch.sh                          # 默认 60s 采一次
#   scripts/macos-mem-watch.sh -i 30                    # 30s 采一次
#   scripts/macos-mem-watch.sh -o /tmp/mem.csv          # 自定义输出
#   scripts/macos-mem-watch.sh --summary /tmp/mem.csv   # 事后总结（每角色单独算斜率）
#
# CSV 列：iso_ts,uptime_sec,pid,role,rss_kb,vsz_kb
# 角色：main / webview / gpu / net
# Ctrl+C 干净退出并打印总结。

set -euo pipefail

INTERVAL=60
OUTFILE=""
MODE="watch"
MAIN_NAME="hindsight"   # case-insensitive 主进程匹配
ETIME_TOL=60            # 秒，helper etimes 与主进程 etimes 差 ≤ 此值就归到本 app

while [[ $# -gt 0 ]]; do
  case "$1" in
    -i|--interval) INTERVAL="$2"; shift 2 ;;
    -o|--out) OUTFILE="$2"; shift 2 ;;
    -n|--name) MAIN_NAME="$2"; shift 2 ;;
    --tol) ETIME_TOL="$2"; shift 2 ;;
    --summary) MODE="summary"; OUTFILE="$2"; shift 2 ;;
    -h|--help) sed -n '2,28p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

# 发现 4 个进程的 PIDs + role；输出 "pid<TAB>role" 每行一个。
# 步骤：
#   1) pgrep -ix 找主 hindsight pid（二进制是 lowercase）
#   2) 单趟扫 ps，把主进程的 etime 和所有 WebKit helper 的 etime 都收集起来
#   3) END 块对比：|etime_helper - etime_main| <= ETIME_TOL 的归属本 app
# 注：macOS BSD ps 只有 etime（MM:SS / HH:MM:SS / DD-HH:MM:SS 三种格式），
# 没有 etimes，需 awk 解析成秒。
discover_pids_with_roles() {
  local main_pid
  main_pid=$(pgrep -ix "$MAIN_NAME" | head -1 || true)
  [[ -z "$main_pid" ]] && return 0

  ps -ax -o pid=,etime=,args= 2>/dev/null \
    | awk -v main_pid="$main_pid" -v tol="$ETIME_TOL" '
        function etime_to_sec(s,   p, d, n) {
          # 入参形如 "MM:SS" / "HH:MM:SS" / "DD-HH:MM:SS"，返回秒数
          d = 0
          if (s ~ /-/) {
            n = split(s, p, "-")
            d = p[1] + 0
            s = p[2]
          }
          n = split(s, p, ":")
          if (n == 3) return d*86400 + p[1]*3600 + p[2]*60 + p[3]
          if (n == 2) return d*86400 +              p[1]*60 + p[2]
          return d*86400 + p[1]
        }
        {
          pid = $1
          et  = etime_to_sec($2)
          if (pid == main_pid) {
            main_et = et
            print pid "\tmain"
            next
          }
          role = ""
          if      ($0 ~ /com\.apple\.WebKit\.WebContent\.xpc/)  role = "webview"
          else if ($0 ~ /com\.apple\.WebKit\.GPU\.xpc/)         role = "gpu"
          else if ($0 ~ /com\.apple\.WebKit\.Networking\.xpc/)  role = "net"
          if (role != "") {
            # 主进程可能在后面才扫到，先缓存所有 helper，END 里再比 etime
            hc++
            h_pid[hc]  = pid
            h_role[hc] = role
            h_et[hc]   = et
          }
        }
        END {
          if (main_et == "") exit  # 主进程不在 ps 输出里？理论上不可能
          for (i = 1; i <= hc; i++) {
            diff = h_et[i] - main_et
            if (diff < 0) diff = -diff
            if (diff <= tol) print h_pid[i] "\t" h_role[i]
          }
        }
      '
}

# 给定 "pid<TAB>role" 列表，逐 PID 采 rss/vsz，输出 "pid\trole\trss_kb\tvsz_kb"
sample_all() {
  local roles_file="$1"
  local pid role stats
  while IFS=$'\t' read -r pid role; do
    [[ -z "$pid" ]] && continue
    stats=$(ps -o rss=,vsz= -p "$pid" 2>/dev/null | awk 'NR==1{print $1"\t"$2}')
    [[ -z "$stats" ]] && continue
    printf "%s\t%s\t%s\n" "$pid" "$role" "$stats"
  done < "$roles_file"
}

# 全 awk 跑总结：每 role 算首末/峰值/斜率，再算总斜率
summarize() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo "no such file: $file" >&2; exit 1
  fi
  awk -F, '
    NR==1 { next }
    {
      ts = $2+0; role = $4; rss = $5+0
      key = ts SUBSEP role
      bucket_ts[key] = ts
      bucket_role[key] = role
      bucket_rss[key] += rss   # 同 ts 同 role 多 pid 求和
    }
    END {
      for (k in bucket_rss) {
        role = bucket_role[k]; ts = bucket_ts[k]; rss = bucket_rss[k]
        if (!(role in first_ts) || ts < first_ts[role]) { first_ts[role]=ts; first_rss[role]=rss }
        if (!(role in last_ts)  || ts > last_ts[role])  { last_ts[role]=ts;  last_rss[role]=rss  }
        if (!(role in peak_rss) || rss > peak_rss[role]) { peak_rss[role]=rss; peak_ts[role]=ts }
        n[role]++
        total_per_ts[ts] += rss
      }
      min_ts = 0; max_ts = 0
      for (t in total_per_ts) {
        if (min_ts==0 || t<min_ts) min_ts=t
        if (t>max_ts) max_ts=t
      }
      dur_min = (max_ts - min_ts) / 60
      total_first = total_per_ts[min_ts]
      total_last  = total_per_ts[max_ts]
      total_delta = total_last - total_first
      total_slope = (dur_min > 0) ? total_delta/dur_min : 0

      printf "duration : %.1f min (%.2f h)\n", dur_min, dur_min/60
      total_ticks = 0
      for (t in total_per_ts) total_ticks++
      printf "samples  : %d ticks\n\n", total_ticks
      printf "%-9s %10s %10s %10s %12s %14s\n", "role", "first_MB", "last_MB", "peak_MB", "delta_MB", "slope_KB/min"
      printf "%-9s %10s %10s %10s %12s %14s\n", "----", "--------", "-------", "-------", "--------", "------------"
      split("webview main gpu net", order, " ")
      for (i=1; i<=4; i++) {
        r = order[i]
        if (!(r in n)) continue
        dt = (last_ts[r] - first_ts[r]) / 60
        ds = last_rss[r] - first_rss[r]
        sl = (dt > 0) ? ds/dt : 0
        printf "%-9s %10.1f %10.1f %10.1f %12.1f %14.2f\n",
               r, first_rss[r]/1024, last_rss[r]/1024, peak_rss[r]/1024, ds/1024, sl
      }
      printf "%-9s %10.1f %10.1f %10s %12.1f %14.2f\n",
             "TOTAL", total_first/1024, total_last/1024, "-", total_delta/1024, total_slope

      print ""
      if ("webview" in n) {
        dt_w = (last_ts["webview"] - first_ts["webview"]) / 60
        sl_w = (dt_w > 0) ? (last_rss["webview"] - first_rss["webview"])/dt_w : 0
        if (sl_w > 100)       printf "verdict (webview): still leaking heavy (%.0f KB/min ~= %.1f MB/h)\n", sl_w, sl_w*60/1024
        else if (sl_w > 20)   printf "verdict (webview): drifting (%.0f KB/min ~= %.1f MB/h)\n", sl_w, sl_w*60/1024
        else                  printf "verdict (webview): flat/noise (%.0f KB/min)\n", sl_w
      }
      if ("main" in n) {
        dt_m = (last_ts["main"] - first_ts["main"]) / 60
        sl_m = (dt_m > 0) ? (last_rss["main"] - first_rss["main"])/dt_m : 0
        if (sl_m > 50)        printf "verdict (main)   : autoreleasepool fix not working (%.0f KB/min)\n", sl_m
        else if (sl_m > 10)   printf "verdict (main)   : slow drift (%.0f KB/min)\n", sl_m
        else                  printf "verdict (main)   : autoreleasepool fix likely effective (%.0f KB/min)\n", sl_m
      }
    }' "$file"
}

if [[ "$MODE" == "summary" ]]; then
  summarize "$OUTFILE"
  exit 0
fi

if [[ -z "$OUTFILE" ]]; then
  OUTFILE="$HOME/hindsight-mem-$(date +%Y%m%d-%H%M%S).csv"
fi

ROLES_TMP=$(mktemp -t hindsight-roles.XXXXXX)
trap 'rm -f "$ROLES_TMP"' EXIT
discover_pids_with_roles > "$ROLES_TMP"
if [[ ! -s "$ROLES_TMP" ]]; then
  echo "no hindsight processes found; start the app first" >&2
  exit 1
fi

echo "iso_ts,uptime_sec,pid,role,rss_kb,vsz_kb" > "$OUTFILE"

echo "discovered processes:"
while IFS=$'\t' read -r pid role; do
  comm=$(ps -o comm= -p "$pid" 2>/dev/null || echo "?")
  printf "  pid=%-6s role=%-7s comm=%s\n" "$pid" "$role" "$comm"
done < "$ROLES_TMP"
echo ""
echo "logging every ${INTERVAL}s to $OUTFILE"
echo "(Ctrl+C to stop and print summary)"
echo ""

START_EPOCH=$(date +%s)
SAMPLE=0
FIRST_TOTAL=""

cleanup() {
  echo ""
  echo "stopped after $SAMPLE samples; log at $OUTFILE"
  echo "---"
  summarize "$OUTFILE" || true
  exit 0
}
trap cleanup INT TERM

while :; do
  # 每 5 tick 重发现：WebKit helpers 偶尔被回收/重启，PID 会变
  if (( SAMPLE % 5 == 0 )) && (( SAMPLE > 0 )); then
    discover_pids_with_roles > "$ROLES_TMP"
    if [[ ! -s "$ROLES_TMP" ]]; then
      echo "all hindsight processes gone; stopping"
      cleanup
    fi
  fi

  NOW_EPOCH=$(date +%s)
  UPTIME=$((NOW_EPOCH - START_EPOCH))
  ISO_TS=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  SAMPLES=$(sample_all "$ROLES_TMP")
  if [[ -z "$SAMPLES" ]]; then
    sleep "$INTERVAL"; continue
  fi

  # 写 CSV
  while IFS=$'\t' read -r pid role rss vsz; do
    [[ -z "$pid" ]] && continue
    echo "$ISO_TS,$UPTIME,$pid,$role,$rss,$vsz" >> "$OUTFILE"
  done <<< "$SAMPLES"

  # awk 聚合：算 total + 每 role 的 MB
  AGG=$(echo "$SAMPLES" | awk -F'\t' '
    { sum[$2] += $3; total += $3 }
    END {
      printf "%d", total
      for (r in sum) printf "|%s=%dM", r, int(sum[r]/1024 + 0.5)
    }
  ')
  TOTAL_KB=${AGG%%|*}
  REST=${AGG#$TOTAL_KB}
  BREAKDOWN=$(echo "$REST" | tr '|' ' ')

  SAMPLE=$((SAMPLE + 1))
  [[ -z "$FIRST_TOTAL" ]] && FIRST_TOTAL=$TOTAL_KB
  DELTA_KB=$((TOTAL_KB - FIRST_TOTAL))
  MINUTES=$(awk -v s="$UPTIME" 'BEGIN{printf "%.1f", s/60}')
  TOTAL_MB=$(awk -v k="$TOTAL_KB" 'BEGIN{printf "%.1f", k/1024}')
  DELTA_MB=$(awk -v k="$DELTA_KB" 'BEGIN{printf "%+.1f", k/1024}')
  if (( UPTIME > 0 )); then
    SLOPE=$(awk -v d="$DELTA_KB" -v u="$UPTIME" 'BEGIN{printf "%+.1f", (d/u)*60}')
  else
    SLOPE="+0.0"
  fi

  printf "[%3d | +%5s min] TOTAL=%6s MB (Δ %s MB, %s KB/min) |%s\n" \
    "$SAMPLE" "$MINUTES" "$TOTAL_MB" "$DELTA_MB" "$SLOPE" "$BREAKDOWN"

  sleep "$INTERVAL"
done
