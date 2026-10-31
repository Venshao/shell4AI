$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

function Assert-ExitCode([string]$Step) {
    if ($LASTEXITCODE -ne 0) {
        throw "$Step 失败，退出码: $LASTEXITCODE"
    }
}

Push-Location (Join-Path $root "src-tauri")
try {
    cargo check --all-targets
    Assert-ExitCode "cargo check"
    cargo test --all-targets
    Assert-ExitCode "cargo test"
}
finally {
    Pop-Location
}

npm run build
Assert-ExitCode "npm run build"

Write-Host "检查通过。" -ForegroundColor Green
