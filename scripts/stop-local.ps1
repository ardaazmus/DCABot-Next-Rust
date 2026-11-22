# Stop only services that are recorded as belonging to this local project.
# The separate PAPER database is intentionally preserved.
$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDirectory = Join-Path $projectRoot '.local-runtime'
$processFile = Join-Path $runtimeDirectory 'processes.json'
$coreBinary = [System.IO.Path]::GetFullPath((Join-Path $projectRoot 'mini-services/dca-core/target/debug/dca_server.exe'))
$nextEntry = (Join-Path $projectRoot 'node_modules/next/dist/bin/next').Replace('/', '\')
$records = @()

if (Test-Path -LiteralPath $processFile) {
    try {
        $records = @(Get-Content -LiteralPath $processFile -Raw | ConvertFrom-Json)
    } catch {
        Write-Output 'Invalid process record; no process will be stopped.'
    }
}

$stopped = 0
foreach ($port in @(3002, 3000)) {
    $listeners = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
    foreach ($listener in $listeners) {
        $processId = [int]$listener.OwningProcess
        $record = $records | Where-Object { [int]$_.Id -eq $processId } | Select-Object -First 1
        $processInfo = Get-CimInstance Win32_Process -Filter ("ProcessId={0}" -f $processId) -ErrorAction SilentlyContinue
        $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
        $matchesRecord = $false

        if ($record -and $processInfo -and $process) {
            try {
                $matchesRecord = [long]$record.Started -eq [long]$process.StartTime.ToUniversalTime().Ticks
            } catch {
                $matchesRecord = $false
            }
        }

        $owned = $false
        if ($matchesRecord -and $port -eq 3002 -and $processInfo.Name -eq 'dca_server.exe') {
            $owned = $processInfo.ExecutablePath -and [System.IO.Path]::GetFullPath($processInfo.ExecutablePath) -ieq $coreBinary
        }
        if ($matchesRecord -and $port -eq 3000 -and $processInfo.Name -eq 'node.exe') {
            $commandLine = [string]$processInfo.CommandLine
            $normalizedCommand = $commandLine.Replace('/', '\')
            $owned = $normalizedCommand.IndexOf($nextEntry, [System.StringComparison]::OrdinalIgnoreCase) -ge 0 -or
                $normalizedCommand.IndexOf('node_modules\next\dist\bin\next', [System.StringComparison]::OrdinalIgnoreCase) -ge 0
        }

        if ($owned) {
            Stop-Process -Id $processId -Force
            Write-Output ("stopped local service PID {0} on port {1}" -f $processId, $port)
            $stopped++
        } else {
            $name = if ($processInfo) { $processInfo.Name } else { 'unknown' }
            Write-Output ("port {0}: left unverified process {1} (PID {2}) untouched" -f $port, $name, $processId)
        }
    }
}

Start-Sleep -Milliseconds 800
$remaining = @(
    Get-NetTCPConnection -State Listen -LocalPort 3000, 3002 -ErrorAction SilentlyContinue
)
if ($remaining.Count -eq 0 -and (Test-Path -LiteralPath $processFile)) {
    Remove-Item -LiteralPath $processFile -ErrorAction SilentlyContinue
}

if ($stopped -eq 0) { Write-Output 'No verified local services were stopped.' }
else { Write-Output 'Done. Paper database preserved.' }
