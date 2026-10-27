# Faz 14-E: stop-local.ps1 — port-dinleyici bazli guvenli kapatma.
# Neden: processes.json kimlik kontrolu (PID+startTime) restart/legacy
# senaryolarinda "identity changed" reddi veriyordu; port-tabanli kapatma
# bunu asar. Guvenlik: yalnizca beklenen surec adlari kapatilir.
$ErrorActionPreference = 'Stop'

$stopped = 0
foreach ($port in @(3002, 3000)) {
    $listeners = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
    foreach ($c in $listeners) {
        $proc = Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
        $name = if ($proc) { $proc.ProcessName } else { '?' }
        $expected = if ($port -eq 3002) { @('dca_server') } else { @('node', 'bun') }
        if ($expected -contains $name) {
            & taskkill.exe /PID $c.OwningProcess /T /F 2>$null | Out-Null
            Write-Output ("stopped {0} (PID {1}) on port {2}" -f $name, $c.OwningProcess, $port)
            $stopped++
        } else {
            Write-Output ("port {0}: unexpected process {1} (PID {2}) — left untouched" -f $port, $name, $c.OwningProcess)
        }
    }
}
Start-Sleep -Milliseconds 800

$processFile = Join-Path (Split-Path -Parent $PSScriptRoot) '.local-runtime/processes.json'
if (Test-Path -LiteralPath $processFile) { Remove-Item -LiteralPath $processFile -ErrorAction SilentlyContinue }

# Paper database is preserved (no DB files touched).
if ($stopped -eq 0) { Write-Output 'No managed local services found.' } else { Write-Output "Done ($stopped stopped). Paper database preserved." }
