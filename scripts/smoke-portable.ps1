param(
    [string]$ExePath = "",
    [int]$TimeoutSeconds = 45
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $ExePath) {
    $ExePath = Join-Path $root "dist\SSH-MCP-portable\SSH MCP.exe"
}
if (-not (Test-Path -LiteralPath $ExePath)) {
    throw "找不到便携版可执行文件: $ExePath"
}

$tempData = Join-Path $env:TEMP ("ssh-mcp-smoke-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tempData | Out-Null
$env:SSH_MCP_DATA_DIR = $tempData

$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$runName = "SSH MCP"
$hadRun = $false
$previousRun = $null
if (Get-ItemProperty -Path $runKey -Name $runName -ErrorAction SilentlyContinue) {
    $hadRun = $true
    $previousRun = (Get-ItemProperty -Path $runKey -Name $runName).$runName
}

$proc = $null
try {
    $proc = Start-Process -FilePath $ExePath -PassThru
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $healthy = $false
    while ((Get-Date) -lt $deadline) {
        try {
            $response = Invoke-WebRequest -Uri "http://127.0.0.1:8765/health" -UseBasicParsing -TimeoutSec 2
            if ($response.StatusCode -eq 200) {
                $healthy = $true
                break
            }
        } catch {
            Start-Sleep -Milliseconds 500
        }
    }
    if (-not $healthy) {
        throw "在 ${TimeoutSeconds}s 内未能访问 /health"
    }
    Write-Host "smoke ok: /health responded" -ForegroundColor Green
}
finally {
    if ($proc -and -not $proc.HasExited) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        try { Wait-Process -Id $proc.Id -Timeout 10 } catch {}
    }
    if ($hadRun) {
        Set-ItemProperty -Path $runKey -Name $runName -Value $previousRun
    } elseif (Get-ItemProperty -Path $runKey -Name $runName -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -Path $runKey -Name $runName -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath $tempData) {
        Remove-Item -LiteralPath $tempData -Recurse -Force -ErrorAction SilentlyContinue
    }
}
