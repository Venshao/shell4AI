param(
    [Parameter(Position = 0)]
    [ValidateSet("tools", "exec", "health")]
    [string]$Action = "tools",
    [string]$HostAlias = "",
    [string]$Command = "hostname",
    [string]$BaseUrl = "http://127.0.0.1:8765",
    [string]$Token = ""
)

$ErrorActionPreference = "Stop"

if (-not $Token) {
    $settingsPath = Join-Path $env:APPDATA "SSH-MCP\settings.json"
    throw "请通过 -Token 传入 GUI 中显示的 MCP Bearer Token。设置文件在: $settingsPath（Token 已加密，不可直接读取）"
}

$headers = @{
    Authorization = "Bearer $Token"
    Accept        = "application/json, text/event-stream"
    "Content-Type" = "application/json"
}

function Invoke-Mcp([hashtable]$Body, [string]$SessionId) {
    $requestHeaders = @{} + $headers
    if ($SessionId) {
        $requestHeaders["mcp-session-id"] = $SessionId
        $requestHeaders["MCP-Protocol-Version"] = "2025-03-26"
    }
    return Invoke-WebRequest -Uri "$BaseUrl/mcp" -Method Post -Headers $requestHeaders -Body ($Body | ConvertTo-Json -Depth 8) -UseBasicParsing
}

function Get-SseJson([string]$Text) {
    foreach ($line in ($Text -split "`n")) {
        if ($line.StartsWith("data: ") -and $line.Substring(6).StartsWith("{")) {
            return ($line.Substring(6) | ConvertFrom-Json)
        }
    }
    throw "响应中未找到 JSON-RPC data 行: $Text"
}

switch ($Action) {
    "health" {
        $response = Invoke-WebRequest -Uri "$BaseUrl/health" -UseBasicParsing
        Write-Output $response.Content
    }
    "tools" {
        $init = Invoke-Mcp @{
            jsonrpc = "2.0"
            id      = 1
            method  = "initialize"
            params  = @{
                protocolVersion = "2025-03-26"
                capabilities    = @{}
                clientInfo      = @{ name = "mcp_smoke"; version = "1.0" }
            }
        } $null
        $session = $init.Headers["mcp-session-id"]
        Invoke-Mcp @{
            jsonrpc = "2.0"
            method  = "notifications/initialized"
        } $session | Out-Null
        $tools = Invoke-Mcp @{
            jsonrpc = "2.0"
            id      = 2
            method  = "tools/list"
            params  = @{}
        } $session
        Get-SseJson $tools.Content | ConvertTo-Json -Depth 10
    }
    "exec" {
        if (-not $HostAlias) { throw "exec 需要 -HostAlias" }
        $init = Invoke-Mcp @{
            jsonrpc = "2.0"
            id      = 1
            method  = "initialize"
            params  = @{
                protocolVersion = "2025-03-26"
                capabilities    = @{}
                clientInfo      = @{ name = "mcp_smoke"; version = "1.0" }
            }
        } $null
        $session = $init.Headers["mcp-session-id"]
        Invoke-Mcp @{
            jsonrpc = "2.0"
            method  = "notifications/initialized"
        } $session | Out-Null
        $result = Invoke-Mcp @{
            jsonrpc = "2.0"
            id      = 3
            method  = "tools/call"
            params  = @{
                name      = "ssh_exec"
                arguments = @{
                    host    = $HostAlias
                    command = $Command
                }
            }
        } $session
        Get-SseJson $result.Content | ConvertTo-Json -Depth 10
    }
}
