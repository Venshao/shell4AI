param(
    [switch]$NoZip
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$sourceExe = Join-Path $root "src-tauri\target\bundle-release\ssh-mcp.exe"
if (-not (Test-Path $sourceExe)) {
    throw "缺少 release 可执行文件。请先运行 scripts\build-standalone.ps1"
}

function Remove-VerifiedDirectory([string]$Path) {
    if (-not (Test-Path $Path)) { return }
    $resolved = (Resolve-Path $Path).Path
    if (-not $resolved.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
        throw "拒绝删除工作区外路径: $resolved"
    }
    [System.IO.Directory]::Delete($resolved, $true)
}

$dist = Join-Path $root "dist"
$portable = Join-Path $dist "SSH-MCP-portable"
$zip = Join-Path $dist "SSH-MCP_0.1.0_x64-portable.zip"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Remove-VerifiedDirectory $portable
New-Item -ItemType Directory -Force -Path $portable | Out-Null

Copy-Item -LiteralPath $sourceExe -Destination (Join-Path $portable "SSH MCP.exe")

@"
SSH MCP Portable
================

1. Double-click "SSH MCP.exe".
2. The application starts with a tray icon.
3. Closing the window hides it to the tray.
4. Use the tray menu to exit and stop the MCP service.
5. Login autostart can be enabled or disabled in the title bar.

This portable build is a single native executable. No Python runtime is required.

User data is stored in:
%APPDATA%\SSH-MCP

MCP endpoint:
http://127.0.0.1:8765/mcp
"@ | Set-Content -LiteralPath (Join-Path $portable "README.txt") -Encoding utf8

if (-not $NoZip) {
    if (Test-Path $zip) { [System.IO.File]::Delete($zip) }
    Compress-Archive -Path (Join-Path $portable "*") -DestinationPath $zip -CompressionLevel Optimal
}

$portableSize = (Get-ChildItem $portable -Recurse -File | Measure-Object Length -Sum).Sum
Write-Host "Portable directory: $portable ($([math]::Round($portableSize / 1MB, 2)) MB)"
if (-not $NoZip) {
    Write-Host "Portable ZIP: $zip ($([math]::Round((Get-Item $zip).Length / 1MB, 2)) MB)"
}
