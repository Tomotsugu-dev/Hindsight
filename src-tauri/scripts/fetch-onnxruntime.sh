#!/usr/bin/env bash
# [离线兜底脚本] 下载 onnxruntime 1.22.x 预编译动态库到 resources/runtime/。
#
# **日常开发不需要跑这个脚本**——运行期会自动从 GitHub release 下载到
# `<data_root>/ai/runtime/`，跟 llama.cpp binary 同款 lazy-fetch。本脚本仅
# 用于离线 / 受限网络环境下的预拉取（拉完后还需要手动复制到 data_root，
# 此脚本默认放到 src-tauri/resources/runtime/ 是给老 build.rs 用的，已废弃）。
#
# 用法：bash src-tauri/scripts/fetch-onnxruntime.sh

set -euo pipefail

VERSION="1.22.0"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNTIME_DIR="$SCRIPT_DIR/../resources/runtime"
mkdir -p "$RUNTIME_DIR"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Darwin)
        case "$ARCH" in
            arm64) ASSET="onnxruntime-osx-arm64-$VERSION.tgz" ;;
            x86_64) ASSET="onnxruntime-osx-x86_64-$VERSION.tgz" ;;
            *) echo "unsupported macOS arch: $ARCH" >&2; exit 1 ;;
        esac
        LIB_NAME_GLOB='libonnxruntime.*.dylib'
        DEST_NAME='libonnxruntime.dylib'
        ;;
    Linux)
        case "$ARCH" in
            x86_64) ASSET="onnxruntime-linux-x64-$VERSION.tgz" ;;
            aarch64) ASSET="onnxruntime-linux-aarch64-$VERSION.tgz" ;;
            *) echo "unsupported Linux arch: $ARCH" >&2; exit 1 ;;
        esac
        LIB_NAME_GLOB='libonnxruntime.so.*'
        DEST_NAME='libonnxruntime.so'
        ;;
    *)
        echo "use fetch-onnxruntime.ps1 on Windows" >&2
        exit 1
        ;;
esac

URL="https://github.com/microsoft/onnxruntime/releases/download/v$VERSION/$ASSET"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "[fetch] downloading $URL"
curl -fL -o "$TMP_DIR/$ASSET" "$URL"

echo "[fetch] extracting"
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR"

INNER="$(find "$TMP_DIR" -maxdepth 1 -type d -name 'onnxruntime-*' | head -1)"
if [ -z "$INNER" ]; then
    echo "extraction layout unexpected" >&2; exit 1
fi

# shellcheck disable=SC2086
LIB_FILE="$(find "$INNER/lib" -maxdepth 1 -name $LIB_NAME_GLOB | head -1)"
if [ -z "$LIB_FILE" ]; then
    echo "lib not found under $INNER/lib" >&2; exit 1
fi

DEST="$RUNTIME_DIR/$DEST_NAME"
cp -f "$LIB_FILE" "$DEST"
echo "[fetch] installed -> $DEST"
