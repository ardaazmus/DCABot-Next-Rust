# Faz 14-E v2: stop-local.ps1 — port-dinleyici bazli guvenli kapatma (PS 5.1 uyumlu, ASCII-only).
$ErrorActionPreference = 'Stop'

$stopped = 0
foreach ($port in @(3002, 3000)) {
    $listeners = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
    foreach ($c in $listeners) {
        $proc = Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
        $name = "?"
        if ($proc) { $name = $proc.ProcessName }
        $ok = $false
        if ($port -eq 3002 -and $name -eq "dca_server") { $ok = $true }
        if ($port -eq 3000 -and ($name -eq "node" -or $name -eq "bun")) { $ok = $true }
        if ($ok) {
            & taskkill.exe /PID $c.OwningProcess /T /F 2>$null | Out-Null
            Write-Output ("stopped {0} (PID {1}) on port {2}" -f $name, $c.OwningProcess, $port)
            $stopped++
        } else {
            Write-Output ("port {0}: unexpected process {1} (PID {2}) left untouched" -f $port, $name, $c.OwningProcess)
        }
    }
}
Start-Sleep -Milliseconds 800

$processFile = Join-Path (Split-Path -Parent $PSScriptRoot) ".local-runtime/processes.json"
if (Test-Path -LiteralPath $processFile) { Remove-Item -LiteralPath $processFile -ErrorAction SilentlyContinue }

if ($stopped -eq 0) { Write-Output "No managed local services found." } else { Write-Output "Done. Paper database preserved." }
