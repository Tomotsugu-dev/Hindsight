#!/usr/bin/env python3
"""云端 VLM 屏幕截图转写实测(OpenAI 兼容端点通用,默认硅基流动)。

目的:验证"云端 VLM 全量转写"方案的三件事——
  1. 转写忠实度(逐字转写 vs 概括幻觉,人工抽查输出)
  2. 真实 token 消耗(usage 字段,不靠估算)
  3. 按真实消耗外推日成本

用法:
  export SILICONFLOW_API_KEY=sk-xxx          # 只从环境变量读,不进命令行历史
  python3 test_siliconflow_vl.py             # 默认:最新一天的截图里等距抽 5 张
  python3 test_siliconflow_vl.py --count 20  # 抽 20 张
  python3 test_siliconflow_vl.py --files a.jpg b.jpg   # 指定文件

结果写入本目录 results_<时间戳>/:每张一个 .txt(完整转写),外加 summary.md。
零第三方依赖:降采样用 macOS 自带 sips,HTTP 用 urllib。
"""

import argparse
import base64
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from datetime import datetime
from pathlib import Path

ENDPOINT_DEFAULT = "https://api.siliconflow.cn/v1/chat/completions"
# 智谱 GLM-4V-Flash 测法:
#   python3 test_siliconflow_vl.py --model glm-4v-flash --key-env ZHIPU_API_KEY \
#     --endpoint https://open.bigmodel.cn/api/paas/v4/chat/completions
# 2026-07 实查目录:Qwen2.5-VL 已下架,现役候选是 Qwen3-VL 系列与两个专用 OCR 模型
# (deepseek-ai/DeepSeek-OCR、PaddlePaddle/PaddleOCR-VL-1.5),用 --model 切换对比。
MODEL_DEFAULT = "Qwen/Qwen3-VL-8B-Instruct"
SCREENSHOTS_ROOT = Path.home() / "Library/Application Support/Hindsight/screenshots"

# 用户指定的提示词方针:逐字转写,没有文字才描述
PROMPT = (
    "逐字转写这张屏幕截图中出现的所有文字,按屏幕上的排版顺序逐行输出。"
    "保持原文语言,不要翻译、不要概括、不要补全被截断的词句。"
    "如果画面中没有任何文字,才用一两句话描述画面内容。"
    "只输出转写结果或描述本身,不要任何前言和解释。"
)

# 挂牌价(元/百万 token,输入/输出),2026-07-19 实查:
# - Qwen3-VL-8B: 国际站 $0.18/$0.68,按 7.2 汇率折算;国内站可能更低,以官网为准
# - PaddleOCR-VL-1.5: 国内站挂牌免费
# 未列出的模型按 Qwen3-VL-8B 价格估算。
PRICE_CNY_PER_M = {
    "Qwen/Qwen3-VL-8B-Instruct": (1.30, 4.90),
    "PaddlePaddle/PaddleOCR-VL-1.5": (0.0, 0.0),
    "glm-4v-flash": (0.0, 0.0),  # 智谱免费档
}
PRICE_FALLBACK = (1.30, 4.90)


def downscale(src: Path, max_side: int, tmpdir: str) -> Path:
    """sips 降采样到长边 max_side,JPEG 输出;失败则原图直传。"""
    dst = Path(tmpdir) / (src.stem + "_small.jpg")
    r = subprocess.run(
        ["sips", "-Z", str(max_side), "-s", "format", "jpeg", str(src), "--out", str(dst)],
        capture_output=True,
    )
    return dst if r.returncode == 0 and dst.exists() else src


def call_vlm(api_key: str, image_path: Path, max_tokens: int, model: str, endpoint: str) -> dict:
    b64 = base64.b64encode(image_path.read_bytes()).decode()
    body = json.dumps(
        {
            "model": model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": PROMPT},
                        {
                            "type": "image_url",
                            "image_url": {"url": f"data:image/jpeg;base64,{b64}"},
                        },
                    ],
                }
            ],
            "max_tokens": max_tokens,
            "temperature": 0,
            "stream": False,
        }
    ).encode()
    req = urllib.request.Request(
        endpoint,
        data=body,
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
    )
    t0 = time.monotonic()
    with urllib.request.urlopen(req, timeout=180) as resp:
        data = json.loads(resp.read())
    latency = time.monotonic() - t0
    usage = data.get("usage", {})
    return {
        "text": data["choices"][0]["message"]["content"],
        "prompt_tokens": usage.get("prompt_tokens", 0),
        "completion_tokens": usage.get("completion_tokens", 0),
        "latency_s": latency,
    }


def pick_default_files(count: int) -> list[Path]:
    """最新一个日期目录里按时间等距抽 count 张。"""
    days = sorted(d for d in SCREENSHOTS_ROOT.iterdir() if d.is_dir())
    if not days:
        sys.exit(f"找不到截图目录: {SCREENSHOTS_ROOT}")
    shots = sorted(days[-1].glob("*.jpg"))
    if not shots:
        sys.exit(f"{days[-1]} 下没有 jpg")
    if len(shots) <= count:
        return shots
    step = len(shots) / count
    return [shots[int(i * step)] for i in range(count)]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--files", nargs="*", type=Path, help="指定截图文件;缺省自动抽样")
    ap.add_argument("--count", type=int, default=5, help="自动抽样张数(默认 5)")
    ap.add_argument("--max-side", type=int, default=1280, help="降采样长边(默认 1280)")
    ap.add_argument("--max-tokens", type=int, default=3000)
    ap.add_argument("--model", default=MODEL_DEFAULT)
    ap.add_argument("--endpoint", default=ENDPOINT_DEFAULT)
    ap.add_argument("--key-env", default="SILICONFLOW_API_KEY", help="API key 的环境变量名")
    args = ap.parse_args()

    api_key = os.environ.get(args.key_env)
    if not api_key:
        sys.exit(f"请先 export {args.key_env}=sk-xxx")

    files = args.files or pick_default_files(args.count)
    # output/ 在 .gitignore 里(转写全文含真实屏幕内容,不入仓)
    out_dir = Path(__file__).parent / "output" / f"vlm_ocr_{datetime.now():%Y%m%d_%H%M%S}"
    out_dir.mkdir(parents=True)

    total_in = total_out = 0
    ok = 0
    lines = []
    with tempfile.TemporaryDirectory() as tmpdir:
        for i, f in enumerate(files, 1):
            small = downscale(f, args.max_side, tmpdir)
            kb_orig, kb_sent = f.stat().st_size // 1024, small.stat().st_size // 1024
            print(f"[{i}/{len(files)}] {f.name} ({kb_orig}KB→{kb_sent}KB) ... ", end="", flush=True)
            try:
                r = call_vlm(api_key, small, args.max_tokens, args.model, args.endpoint)
            except urllib.error.HTTPError as e:
                detail = e.read().decode(errors="replace")[:300]
                print(f"HTTP {e.code}: {detail}")
                lines.append(f"| {f.name} | 失败 HTTP {e.code} | - | - | - |")
                continue
            except Exception as e:  # noqa: BLE001 一张失败不中断整批
                print(f"失败: {e}")
                lines.append(f"| {f.name} | 失败 {type(e).__name__} | - | - | - |")
                continue
            ok += 1
            total_in += r["prompt_tokens"]
            total_out += r["completion_tokens"]
            (out_dir / f"{i:03d}_{f.stem}.txt").write_text(
                f"# 源: {f}\n# in={r['prompt_tokens']} out={r['completion_tokens']} "
                f"latency={r['latency_s']:.1f}s\n\n{r['text']}\n",
                encoding="utf-8",
            )
            print(
                f"in={r['prompt_tokens']} out={r['completion_tokens']} "
                f"{r['latency_s']:.1f}s | {r['text'][:40]!r}..."
            )
            lines.append(
                f"| {f.name} | ok | {r['prompt_tokens']} | "
                f"{r['completion_tokens']} | {r['latency_s']:.1f}s |"
            )

    if ok == 0:
        sys.exit("全部失败,不出报告")

    avg_in, avg_out = total_in / ok, total_out / ok
    price_in, price_out = PRICE_CNY_PER_M.get(args.model, PRICE_FALLBACK)
    cost = total_in / 1e6 * price_in + total_out / 1e6 * price_out
    per_frame = cost / ok
    # 外推:用你库里数出来的产量档位
    proj = {n: per_frame * n for n in (500, 1000, 1500, 2000)}
    summary = (
        f"# 实测汇总 {datetime.now():%Y-%m-%d %H:%M}\n\n"
        f"模型: {args.model} @ {args.endpoint}\n"
        f"降采样长边: {args.max_side}px | 挂牌价(需核对): 入¥{price_in}/M 出¥{price_out}/M\n\n"
        f"| 文件 | 状态 | in tok | out tok | 延迟 |\n|---|---|---|---|---|\n"
        + "\n".join(lines)
        + f"\n\n成功 {ok}/{len(files)} 张 | 平均 in={avg_in:.0f} out={avg_out:.0f} tok/张\n"
        f"本次合计 {total_in + total_out} tok ≈ ¥{cost:.4f}(¥{per_frame:.5f}/张)\n\n"
        f"## 日成本外推(按本次单张均值)\n\n"
        + "\n".join(f"- {n} 张/天 ≈ ¥{c:.2f}/天,¥{c * 30:.0f}/月" for n, c in proj.items())
        + "\n\n忠实度请人工抽查 results 目录里的转写全文,对照原图看有无概括/编造。\n"
    )
    (out_dir / "summary.md").write_text(summary, encoding="utf-8")
    print(f"\n{'=' * 50}\n{summary}\n结果目录: {out_dir}")


if __name__ == "__main__":
    main()
