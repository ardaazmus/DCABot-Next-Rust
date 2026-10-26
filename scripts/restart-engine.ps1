# Engine-only restart (UI on :3000 keeps running).
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\restart-engine.ps1
# NOTE: keep this file ASCII-only - PS 5.1 reads BOM-less UTF-8 as ANSI.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDirectory = Join-Path $projectRoot '.local-runtime'
$coreBinary = Join-Path $projectRoot 'mini-services/dca-core/target/debug/dca_server.exe'

# Keys: same persistent file as start-local.ps1.
$keysFile = Join-Path $runtimeDirectory 'local-keys.env'
if (-not (Test-Path -LiteralPath $keysFile)) { throw "local-keys.env yok - once npm run local:start calistirin" }
Get-Content -LiteralPath $keysFile | ForEach-Object {
    $line = $_.TrimStart([char]0xFEFF)
    if ($line -cmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
        Set-Item -Path ('env:' + $Matches[1]) -Value $Matches[2]
    }
}
if (-not $env:DCA_API_KEY -or -not $env:DCA_VAULT_KEY) { throw "anahtarlar okunamadi" }

$old = Get-NetTCPConnection -State Listen -LocalPort 3002 -ErrorAction SilentlyContinue
if ($old) {
    foreach ($c in $old) {
        & taskkill.exe /PID $c.OwningProcess /T /F 2>$null | Out-Null
    }
    Start-Sleep -Milliseconds 800
}

$env:DCA_DB_PATH = Join-Path $runtimeDirectory 'paper.sqlite3'
$env:DCA_BYBIT_TESTNET = '1'
$env:RUST_LOG = 'info'
$env:NEXT_TELEMETRY_DISABLED = '1'

$proc = Start-Process -FilePath $coreBinary -WorkingDirectory $runtimeDirectory -WindowStyle Hidden -PassThru `
    -RedirectStandardOutput (Join-Path $runtimeDirectory 'core.stdout.log') `
    -RedirectStandardError (Join-Path $runtimeDirectory 'core.stderr.log')

$deadline = [DateTime]::UtcNow.AddSeconds(20)
do {
    Start-Sleep -Milliseconds 300
    $null = & curl.exe --noproxy '*' --silent --fail --max-time 5 http://127.0.0.1:3002/api/health 2>$null
} until ($LASTEXITCODE -eq 0 -or [DateTime]::UtcNow -gt $deadline)
if ($LASTEXITCODE -ne 0) { throw "engine hazir olmadi - core.stderr.log dosyasina bakin" }
Write-Output "engine restarted (PID $($proc.Id))"
