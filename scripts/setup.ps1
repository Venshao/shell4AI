param(
    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

function Assert-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "缺少命令: $Name"
    }
}

function Assert-ExitCode([string]$Step) {
    if ($LASTEXITCODE -ne 0) {
        throw "$Step 失败，退出码: $LASTEXITCODE"
    }
}

Assert-Command node
Assert-Command npm
Assert-Command rustc
Assert-Command cargo

Write-Host "Node $(node -v)"
Write-Host "npm $(npm -v)"
Write-Host "$(rustc --version)"
Write-Host "$(cargo --version)"

npm install
Assert-ExitCode "安装 npm 依赖"

Push-Location (Join-Path $root "src-tauri")
try {
    cargo fetch
    Assert-ExitCode "拉取 Rust 依赖"
    if (-not $SkipTests) {
        cargo test --all-targets
        Assert-ExitCode "运行 Rust 测试"
    }
}
finally {
    Pop-Location
}

npm run build
Assert-ExitCode "构建前端"

Write-Host "环境就绪。运行 scripts\run-dev.ps1 启动应用。" -ForegroundColor Green
