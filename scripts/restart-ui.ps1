# UI-only restart (engine on :3002 keeps running).
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\restart-ui.ps1
# NOTE: keep this file ASCII-only - PS 5.1 reads BOM-less UTF-8 as ANSI.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDirectory = Join-Path $projectRoot '.local-runtime'

# Keys: same persistent file as start-local.ps1 (NEXT_PUBLIC_ for the browser bundle).
$keysFile = Join-Path $runtimeDirectory 'local-keys.env'
if (Test-Path -LiteralPath $keysFile) {
    Get-Content -LiteralPath $keysFile | ForEach-Object {
        $line = $_.TrimStart([char]0xFEFF)
        if ($line -cmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
            Set-Item -Path ('env:' + $Matches[1]) -Value $Matches[2]
        }
    }
}

$old = Get-NetTCPConnection -State Listen -LocalPort 3000 -ErrorAction SilentlyContinue
if ($old) {
    foreach ($c in $old) {
        & taskkill.exe /PID $c.OwningProcess /T /F 2>$null | Out-Null
    }
    Start-Sleep -Milliseconds 800
}

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
if ($LASTEXITCODE -ne 0) { throw "ui hazir olmadi - ui.stderr.log dosyasina bakin" }
Write-Output "ui restarted (PID $($proc.Id))"
