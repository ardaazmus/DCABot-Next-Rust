# Engine-only restart (UI on :3000 keeps running).
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\restart-engine.ps1
# NOTE: keep this file ASCII-only - PS 5.1 reads BOM-less UTF-8 as ANSI.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDirectory = Join-Path $projectRoot '.local-runtime'
$coreBinary = Join-Path $projectRoot 'mini-services/dca-core/target/debug/dca_server.exe'
$processFile = Join-Path $runtimeDirectory 'processes.json'
$recordHelpers = Join-Path $PSScriptRoot 'process-records.ps1'
. $recordHelpers

# Keys: same persistent file as start-local.ps1. Read only the engine's
# credentials and never import the whole secret file into this process env.
$keysFile = Join-Path $runtimeDirectory 'local-keys.env'
if (-not (Test-Path -LiteralPath $keysFile)) { throw "local-keys.env yok - once npm run local:start calistirin" }
$keyValues = @{}
Protect-LocalRuntimeDirectory -Path $runtimeDirectory
Protect-LocalSecretFile -Path $keysFile
foreach ($lineRaw in [System.IO.File]::ReadAllLines($keysFile)) {
    $line = $lineRaw.TrimStart([char]0xFEFF)
    if ($line -cmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
        $keyValues[$Matches[1]] = $Matches[2]
    }
}
if (-not $keyValues['DCA_API_KEY'] -or -not $keyValues['DCA_VAULT_KEY']) { throw "anahtarlar okunamadi" }

$records = Read-LocalProcessRecords -Path $processFile
$expectedBinary = [System.IO.Path]::GetFullPath($coreBinary)
$old = Get-NetTCPConnection -State Listen -LocalPort 3002 -ErrorAction SilentlyContinue
foreach ($listener in $old) {
    $processId = [int]$listener.OwningProcess
    $record = $records | Where-Object { [int]$_.Id -eq $processId -and $_.Name -eq 'dca_server' } | Select-Object -First 1
    $processInfo = Get-CimInstance Win32_Process -Filter ("ProcessId={0}" -f $processId) -ErrorAction SilentlyContinue
    $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
    $matchesRecord = $false
    if ($record -and $processInfo -and $process -and $processInfo.Name -eq 'dca_server.exe') {
        try { $matchesRecord = [long]$record.Started -eq [long]$process.StartTime.ToUniversalTime().Ticks } catch { $matchesRecord = $false }
    }
    if (-not $matchesRecord -or -not $processInfo.ExecutablePath -or
        [System.IO.Path]::GetFullPath($processInfo.ExecutablePath) -ine $expectedBinary) {
        throw ("Port 3002 is occupied by an unverified process (PID {0}); refusing to stop it." -f $processId)
    }
    Stop-Process -Id $processId -Force
}
if ($old) { Start-Sleep -Milliseconds 800 }

$env:DCA_DB_PATH = Join-Path $runtimeDirectory 'paper.sqlite3'
$env:DCA_BYBIT_TESTNET = '1'
$env:DCA_OKX_DEMO = '1'
$env:DCA_BINANCE_TESTNET = '1'
$env:BINANCE_SPOT_BASE = 'https://testnet.binance.vision'
$env:BINANCE_FUTURES_TESTNET_BASE = 'https://testnet.binancefuture.com'
$env:RUST_LOG = 'info'
$env:NEXT_TELEMETRY_DISABLED = '1'
# The engine must not inherit UI session signing or browser-proxy credentials.
Remove-Item Env:DCA_SESSION_SECRET -ErrorAction SilentlyContinue
Remove-Item Env:ENGINE_API_KEY -ErrorAction SilentlyContinue
$env:DCA_API_KEY = $keyValues['DCA_API_KEY']
$env:DCA_VAULT_KEY = $keyValues['DCA_VAULT_KEY']

$proc = Start-Process -FilePath $coreBinary -WorkingDirectory $runtimeDirectory -WindowStyle Hidden -PassThru `
    -RedirectStandardOutput (Join-Path $runtimeDirectory 'core.stdout.log') `
    -RedirectStandardError (Join-Path $runtimeDirectory 'core.stderr.log')
Remove-Item Env:DCA_API_KEY -ErrorAction SilentlyContinue
Remove-Item Env:DCA_VAULT_KEY -ErrorAction SilentlyContinue

$deadline = [DateTime]::UtcNow.AddSeconds(20)
do {
    Start-Sleep -Milliseconds 300
    $null = & curl.exe --noproxy '*' --silent --fail --max-time 5 http://127.0.0.1:3002/api/health 2>$null
} until ($LASTEXITCODE -eq 0 -or [DateTime]::UtcNow -gt $deadline)
if ($LASTEXITCODE -ne 0) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    throw "engine hazir olmadi - core.stderr.log dosyasina bakin"
}

New-Item -ItemType Directory -Force -Path $runtimeDirectory | Out-Null
$started = Get-Process -Id $proc.Id
$records = @($records | Where-Object { $_.Name -ne 'dca_server' })
$records += @{ Id = $proc.Id; Name = $proc.ProcessName; Started = $started.StartTime.ToUniversalTime().Ticks }
Write-LocalProcessRecords -Path $processFile -Records $records
Write-Output "engine restarted (PID $($proc.Id))"
