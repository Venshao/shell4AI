$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    throw "缺少 npm"
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "缺少 cargo，请先安装 Rust MSVC 工具链"
}

npm run tauri -- dev
