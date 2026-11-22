# UI-only restart (engine on :3002 keeps running).
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\restart-ui.ps1
# NOTE: keep this file ASCII-only - PS 5.1 reads BOM-less UTF-8 as ANSI.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDirectory = Join-Path $projectRoot '.local-runtime'
$processFile = Join-Path $runtimeDirectory 'processes.json'
$recordHelpers = Join-Path $PSScriptRoot 'process-records.ps1'
. $recordHelpers

# Keys: same persistent server-side file as start-local.ps1.
$keysFile = Join-Path $runtimeDirectory 'local-keys.env'
if (Test-Path -LiteralPath $keysFile) {
    Protect-LocalRuntimeDirectory -Path $runtimeDirectory
    Protect-LocalSecretFile -Path $keysFile
    $keyValues = @{}
    foreach ($keyLine in [System.IO.File]::ReadAllLines($keysFile)) {
        $line = $keyLine.TrimStart([char]0xFEFF)
        if ($line -cmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') { $keyValues[$Matches[1]] = $Matches[2] }
    }
} else {
    throw "local-keys.env yok: $keysFile"
}
if (-not $keyValues['DCA_API_KEY'] -or -not $keyValues['DCA_SESSION_SECRET']) { throw 'local-keys.env UI anahtarlari eksik' }
Remove-Item Env:DCA_API_KEY -ErrorAction SilentlyContinue
Remove-Item Env:DCA_VAULT_KEY -ErrorAction SilentlyContinue
$env:ENGINE_API_KEY = $keyValues['DCA_API_KEY']
$env:DCA_SESSION_SECRET = $keyValues['DCA_SESSION_SECRET']

$records = Read-LocalProcessRecords -Path $processFile
$nextEntry = (Join-Path $projectRoot 'node_modules/next/dist/bin/next').Replace('/', '\')
$nextServerEntry = (Join-Path $projectRoot 'node_modules/next/dist/server/lib/start-server.js').Replace('/', '\')
$old = Get-NetTCPConnection -State Listen -LocalPort 3000 -ErrorAction SilentlyContinue
$handledListenerIds = @{}
foreach ($listener in $old) {
    $processId = [int]$listener.OwningProcess
    if ($handledListenerIds.ContainsKey([string]$processId)) { continue }
    $handledListenerIds[[string]$processId] = $true
    $nextPair = Get-VerifiedLocalNextProcessPair -ListenerProcessId $processId -Records $records -ProjectRoot $projectRoot
    if ($nextPair) {
        Stop-Process -Id $nextPair.ChildId -Force -ErrorAction SilentlyContinue
        Stop-Process -Id $nextPair.ParentId -Force -ErrorAction SilentlyContinue
        continue
    }
    $record = $records | Where-Object { [int]$_.Id -eq $processId } | Select-Object -First 1
    $processInfo = Get-CimInstance Win32_Process -Filter ("ProcessId={0}" -f $processId) -ErrorAction SilentlyContinue
    $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
    $matchesRecord = $false
    if ($record -and $processInfo -and $process -and $processInfo.Name -eq 'node.exe') {
        try { $matchesRecord = [long]$record.Started -eq [long]$process.StartTime.ToUniversalTime().Ticks } catch { $matchesRecord = $false }
    }
    $normalizedCommand = ([string]$processInfo.CommandLine).Replace('/', '\')
    $isThisNext = $normalizedCommand.IndexOf($nextEntry, [System.StringComparison]::OrdinalIgnoreCase) -ge 0 -or
        $normalizedCommand.IndexOf('node_modules\next\dist\bin\next', [System.StringComparison]::OrdinalIgnoreCase) -ge 0 -or
        $normalizedCommand.IndexOf($nextServerEntry, [System.StringComparison]::OrdinalIgnoreCase) -ge 0
    if (-not $matchesRecord -or -not $isThisNext) {
        throw ("Port 3000 is occupied by an unverified process (PID {0}); refusing to stop it." -f $processId)
    }
    Stop-Process -Id $processId -Force
}
if ($old) { Start-Sleep -Milliseconds 800 }

$env:NODE_OPTIONS = '--max-old-space-size=1024'
$env:RAYON_NUM_THREADS = '2'
$env:DCA_LOCAL_RUN = '1'
$env:NEXT_TELEMETRY_DISABLED = '1'
$nodeBinary = (Get-Command node -ErrorAction Stop).Source
$proc = Start-Process -FilePath $nodeBinary -ArgumentList @('node_modules/next/dist/bin/next', 'dev', '--hostname', '127.0.0.1', '--port', '3000', '--webpack') -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru `
    -RedirectStandardOutput (Join-Path $runtimeDirectory 'ui.stdout.log') `
    -RedirectStandardError (Join-Path $runtimeDirectory 'ui.stderr.log')

$deadline = [DateTime]::UtcNow.AddSeconds(150)
do {
    Start-Sleep -Milliseconds 500
    $null = & curl.exe --noproxy '*' --silent --fail --max-time 30 http://127.0.0.1:3000/ 2>$null
} until ($LASTEXITCODE -eq 0 -or [DateTime]::UtcNow -gt $deadline)
if ($LASTEXITCODE -ne 0) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    throw "ui hazir olmadi - ui.stderr.log dosyasina bakin"
}

New-Item -ItemType Directory -Force -Path $runtimeDirectory | Out-Null
$process = Get-Process -Id $proc.Id
$records = @($records | Where-Object { $_.Name -ne 'node' })
$records += @{ Id = $proc.Id; Name = $proc.ProcessName; Started = $process.StartTime.ToUniversalTime().Ticks }
Write-LocalProcessRecords -Path $processFile -Records $records
Remove-Item Env:ENGINE_API_KEY -ErrorAction SilentlyContinue
Remove-Item Env:DCA_SESSION_SECRET -ErrorAction SilentlyContinue
Write-Output "ui restarted (PID $($proc.Id))"
