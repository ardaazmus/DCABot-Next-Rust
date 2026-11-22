$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDirectory = Join-Path $projectRoot '.local-runtime'
$coreBinary = Join-Path $projectRoot 'mini-services/dca-core/target/debug/dca_server.exe'
$processFile = Join-Path $runtimeDirectory 'processes.json'
$recordHelpers = Join-Path $PSScriptRoot 'process-records.ps1'
. $recordHelpers

$keysFile = Join-Path $runtimeDirectory 'local-keys.env'
if (-not (Test-Path -LiteralPath $keysFile)) { throw "local-keys.env yok: $keysFile" }
Protect-LocalRuntimeDirectory -Path $runtimeDirectory
Protect-LocalSecretFile -Path $keysFile
$keyValues = @{}
foreach ($keyLine in [System.IO.File]::ReadAllLines($keysFile)) {
    $line = $keyLine.TrimStart([char]0xFEFF)
    if ($line -cmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') { $keyValues[$Matches[1]] = $Matches[2] }
}
if (-not $keyValues['DCA_API_KEY'] -or -not $keyValues['DCA_VAULT_KEY']) { throw 'local-keys.env motor anahtarlari eksik' }

$records = Read-LocalProcessRecords -Path $processFile
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
    $expectedBinary = [System.IO.Path]::GetFullPath($coreBinary)
    $actualBinary = [string]$processInfo.ExecutablePath
    if (-not $matchesRecord -or -not $actualBinary -or
        -not [string]::Equals([System.IO.Path]::GetFullPath($actualBinary), $expectedBinary, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw ("Port 3002 is occupied by an unverified process (PID {0}); refusing to stop it." -f $processId)
    }
    & taskkill.exe /PID $processId /T /F 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) { throw ("Verified local engine process could not be stopped (PID {0})." -f $processId) }
}
if ($old) { Start-Sleep -Milliseconds 800 }

# The foreground engine receives only its own API and vault credentials.
Remove-Item Env:DCA_SESSION_SECRET -ErrorAction SilentlyContinue
Remove-Item Env:ENGINE_API_KEY -ErrorAction SilentlyContinue
$env:DCA_API_KEY = $keyValues['DCA_API_KEY']
$env:DCA_VAULT_KEY = $keyValues['DCA_VAULT_KEY']
$env:DCA_DB_PATH = Join-Path $runtimeDirectory 'paper.sqlite3'
$env:DCA_BYBIT_TESTNET = '1'
$env:DCA_OKX_DEMO = '1'
$env:DCA_BINANCE_TESTNET = '1'
$env:BINANCE_SPOT_BASE = 'https://testnet.binance.vision'
$env:BINANCE_FUTURES_TESTNET_BASE = 'https://testnet.binancefuture.com'
$env:RUST_LOG = 'info'
$env:NEXT_TELEMETRY_DISABLED = '1'

Set-Location $runtimeDirectory
& $coreBinary
$engineExitCode = $LASTEXITCODE
Remove-Item Env:DCA_API_KEY -ErrorAction SilentlyContinue
Remove-Item Env:DCA_VAULT_KEY -ErrorAction SilentlyContinue
exit $engineExitCode
