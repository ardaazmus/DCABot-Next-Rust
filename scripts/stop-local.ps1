$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$processFile = Join-Path $projectRoot '.local-runtime/processes.json'

if (-not (Test-Path -LiteralPath $processFile)) {
    Write-Output 'No managed local services found.'
    return
}

$savedProcesses = @(Get-Content -LiteralPath $processFile -Raw | ConvertFrom-Json)
foreach ($savedProcess in $savedProcesses) {
    $runningProcess = Get-Process -Id $savedProcess.Id -ErrorAction SilentlyContinue
    if ($null -eq $runningProcess) {
        continue
    }
    if ($runningProcess.ProcessName -ne $savedProcess.Name -or $runningProcess.StartTime.ToUniversalTime().Ticks -ne $savedProcess.Started) {
        throw "Process identity changed for PID $($savedProcess.Id); refusing to stop it."
    }
    & taskkill.exe /PID $runningProcess.Id /T /F | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Could not stop local process $($runningProcess.Id)."
    }
}

Remove-Item -LiteralPath $processFile
Write-Output 'Local services stopped. Paper database is preserved.'
