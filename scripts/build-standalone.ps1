$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

function Assert-ExitCode([string]$Step) {
    if ($LASTEXITCODE -ne 0) {
        throw "$Step 失败，退出码: $LASTEXITCODE"
    }
}

npm run build
Assert-ExitCode "构建前端"

npx tauri build --config src-tauri\tauri.bundle.json -- --profile bundle-release
Assert-ExitCode "构建 Tauri release"

$exe = Join-Path $root "src-tauri\target\bundle-release\shell4AI.exe"
$nsis = Join-Path $root "src-tauri\target\bundle-release\bundle\nsis"
Write-Host "Release exe: $exe"
if (Test-Path $nsis) {
    Get-ChildItem $nsis -Filter *.exe | ForEach-Object {
        Write-Host "NSIS installer: $($_.FullName)"
    }
}
