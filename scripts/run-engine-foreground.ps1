$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDirectory = Join-Path $projectRoot '.local-runtime'
$coreBinary = Join-Path $projectRoot 'mini-services/dca-core/target/debug/dca_server.exe'

$keysFile = Join-Path $runtimeDirectory 'local-keys.env'
if (-not (Test-Path -LiteralPath $keysFile)) { throw "local-keys.env yok" }
Get-Content -LiteralPath $keysFile | ForEach-Object {
    $line = $_.TrimStart([char]0xFEFF)
    if ($line -cmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
        Set-Item -Path ('env:' + $Matches[1]) -Value $Matches[2]
    }
}

$old = Get-NetTCPConnection -State Listen -LocalPort 3002 -ErrorAction SilentlyContinue
if ($old) {
    foreach ($c in $old) {
        & taskkill.exe /PID $c.OwningProcess /T /F 2>$null | Out-Null
    }
    Start-Sleep -Milliseconds 800
}

$env:DCA_DB_PATH = Join-Path $runtimeDirectory 'paper.sqlite3'
$env:DCA_BYBIT_TESTNET = '1'
$env:DCA_OKX_DEMO = '1'
$env:RUST_LOG = 'info'
$env:NEXT_TELEMETRY_DISABLED = '1'

Set-Location $runtimeDirectory
& $coreBinary
