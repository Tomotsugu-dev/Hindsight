# 一次性脚本：下载 onnxruntime 1.22.x 预编译 DLL 到 resources/runtime/。
# 跑一次即可。文件 ~30 MB，不进 git（.gitignore 已排除）。
#
# 用法：pwsh src-tauri/scripts/fetch-onnxruntime.ps1

$ErrorActionPreference = 'Stop'

# 跟 ort-sys 2.0.0-rc.10 的 ORT_API_VERSION = 22 对齐（即 ORT 1.22.x）
$Version = '1.22.0'
$AssetName = "onnxruntime-win-x64-$Version.zip"
$Url = "https://github.com/microsoft/onnxruntime/releases/download/v$Version/$AssetName"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RuntimeDir = Join-Path $ScriptDir '..\resources\runtime'
$RuntimeDir = (Resolve-Path -LiteralPath $RuntimeDir).Path
$TmpZip = Join-Path $env:TEMP $AssetName
$TmpExtract = Join-Path $env:TEMP "ort-extract-$Version"

Write-Host "[fetch] downloading $Url"
Invoke-WebRequest -Uri $Url -OutFile $TmpZip

Write-Host "[fetch] extracting to $TmpExtract"
if (Test-Path $TmpExtract) { Remove-Item -Recurse -Force $TmpExtract }
Expand-Archive -Path $TmpZip -DestinationPath $TmpExtract

$Inner = Get-ChildItem -Directory $TmpExtract | Select-Object -First 1
$Dll = Join-Path $Inner.FullName 'lib\onnxruntime.dll'
if (-not (Test-Path $Dll)) {
    throw "expected onnxruntime.dll at $Dll, not found"
}

$Dest = Join-Path $RuntimeDir 'onnxruntime.dll'
Copy-Item -Force $Dll $Dest
Write-Host "[fetch] installed -> $Dest"

# 清理临时
Remove-Item -Force $TmpZip
Remove-Item -Recurse -Force $TmpExtract
Write-Host '[fetch] done.'
