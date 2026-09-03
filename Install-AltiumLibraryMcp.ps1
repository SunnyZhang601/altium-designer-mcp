[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory = $true)]
    [string]$LibraryRoot,

    [string]$NodePath
)

$ErrorActionPreference = 'Stop'

$packageRoot = $PSScriptRoot
$launcherPath = Join-Path $packageRoot 'scripts\run-wasi.mjs'
$wasmPath = Join-Path $packageRoot 'dist\altium-designer-mcp.wasm'

foreach ($path in @($launcherPath, $wasmPath, $LibraryRoot)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required path does not exist: $path"
    }
}

if (-not $NodePath) {
    $nodeCommand = Get-Command node.exe -ErrorAction SilentlyContinue
    if (-not $nodeCommand) {
        $nodeCommand = Get-Command node -ErrorAction SilentlyContinue
    }
    if (-not $nodeCommand) {
        throw 'Node.js was not found. Install Node.js, or rerun with -NodePath <node.exe path>.'
    }
    $NodePath = $nodeCommand.Source
}

if (-not (Test-Path -LiteralPath $NodePath)) {
    throw "Node.js executable does not exist: $NodePath"
}

$configDirectory = Join-Path $env:USERPROFILE '.altium-designer-mcp'
$configPath = Join-Path $configDirectory 'config.json'
$mcpDirectory = Join-Path $env:APPDATA 'Code\User'
$mcpPath = Join-Path $mcpDirectory 'mcp.json'

$libraryConfig = [ordered]@{
    allowed_paths = @('/libraries')
    logging = [ordered]@{
        level = 'warn'
        audit_log_path = '/config/audit.jsonl'
    }
    rate_limit = [ordered]@{
        max_burst = 120
        refill_per_sec = 30.0
    }
}

if (Test-Path -LiteralPath $mcpPath) {
    $mcpConfig = Get-Content -LiteralPath $mcpPath -Raw | ConvertFrom-Json
} else {
    $mcpConfig = [pscustomobject]@{ servers = [pscustomobject]@{} }
}

if (-not $mcpConfig.servers) {
    $mcpConfig | Add-Member -MemberType NoteProperty -Name servers -Value ([pscustomobject]@{})
}

$serverConfig = [pscustomobject]@{
    type = 'stdio'
    command = (Resolve-Path -LiteralPath $NodePath).Path
    args = @(
        (Resolve-Path -LiteralPath $launcherPath).Path,
        (Resolve-Path -LiteralPath $wasmPath).Path,
        $configPath,
        (Resolve-Path -LiteralPath $LibraryRoot).Path
    )
}

if ($mcpConfig.servers.PSObject.Properties['altium-library-mcp']) {
    $mcpConfig.servers.'altium-library-mcp' = $serverConfig
} else {
    $mcpConfig.servers | Add-Member -MemberType NoteProperty -Name 'altium-library-mcp' -Value $serverConfig
}

if ($PSCmdlet.ShouldProcess($configPath, 'Write WASI library MCP configuration')) {
    New-Item -ItemType Directory -Path $configDirectory -Force | Out-Null
    $libraryConfig | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $configPath -Encoding UTF8
}

if ($PSCmdlet.ShouldProcess($mcpPath, 'Register Altium Library MCP in VS Code')) {
    New-Item -ItemType Directory -Path $mcpDirectory -Force | Out-Null
    $mcpConfig | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $mcpPath -Encoding UTF8
}

Write-Output 'Altium Library MCP WASI configuration is ready.'
Write-Output "Library root mapped to /libraries: $((Resolve-Path -LiteralPath $LibraryRoot).Path)"
Write-Output 'Reload VS Code, then use /libraries/<relative path> in Library MCP tool calls.'