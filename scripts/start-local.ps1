param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDirectory = Join-Path $projectRoot '.local-runtime'
$coreDirectory = Join-Path $projectRoot 'mini-services/dca-core'
$coreBinary = Join-Path $coreDirectory 'target/debug/dca_server.exe'
$processFile = Join-Path $runtimeDirectory 'processes.json'

# C-2 / M-7: motor (SEC-01/SEC-02) DCA_API_KEY + DCA_VAULT_KEY zorunlu tutar;
# UI sunucu proxy'si (P0-9) ENGINE_API_KEY / DCA_API_KEY kullanır. Anahtarlar İLK çalıştırmada
# rastgele üretilir ve .local-runtime/local-keys.env içinde kalıcıdır —
# sonraki başlatmalar aynı anahtarla açılır (vault şifresi değişmez).
# Sabit/bilinen dev anahtar dönemi kapandı. local-keys.env yalnız yerel
# çalışma içindir; depoya girmez (gitignore: .local-runtime).
$keysFile = Join-Path $runtimeDirectory 'local-keys.env'
if (-not (Test-Path -LiteralPath $keysFile)) {
    New-Item -ItemType Directory -Force -Path $runtimeDirectory | Out-Null
    $api  = ('k-' + [System.Guid]::NewGuid().ToString('N') + [System.Guid]::NewGuid().ToString('N'))
    $vault = ('v-' + [System.Guid]::NewGuid().ToString('N') + [System.Guid]::NewGuid().ToString('N'))
    # BOM'suz yazim: PowerShell 5.1'in `Set-Content -Encoding UTF8`'i BOM ekler,
    # BOM ise ilk satirin regex eslesmesini bozar. Anahtarlar ASCII'dir.
    [System.IO.File]::WriteAllLines($keysFile, @(
        "DCA_API_KEY=$api",
        "DCA_VAULT_KEY=$vault"
    ))
    Write-Output "Generated local auth keys: $keysFile"
}
Get-Content -LiteralPath $keysFile | ForEach-Object {
    $line = $_.TrimStart([char]0xFEFF)
    # -cmatch: PS 5.1 -match'i IgnoreCase + kültür-duyarlıdır; Türkçe kültürde
    # [A-Za-z] sınıf aralıkları yanlış eşleşir (canlı teşhis: match=False,
    # cmatch=True). Anahtar adları ASCII olduğundan cmatch güvenlidir.
    if ($line -cmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
        Set-Item -Path ('env:' + $Matches[1]) -Value $Matches[2]
    }
}
if (-not $env:DCA_API_KEY -or -not $env:DCA_VAULT_KEY) {
    throw "local-keys.env okunamadi veya eksik alan var: $keysFile"
}
$env:ENGINE_API_KEY = $env:DCA_API_KEY

foreach ($port in @(3000, 3002)) {
    $listener = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
    if ($listener) {
        throw "Port $port is already in use. Stop the existing local services first."
    }
}

New-Item -ItemType Directory -Force -Path $runtimeDirectory | Out-Null
$env:CARGO_BUILD_JOBS = '1'
$env:CARGO_INCREMENTAL = '0'

if (-not $SkipBuild) {
    Push-Location $coreDirectory
    try {
        & cargo build --locked --offline --bin dca_server
        if ($LASTEXITCODE -ne 0) {
            throw 'Rust build failed. No local services were started.'
        }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $coreBinary)) {
    throw "Engine binary is missing: $coreBinary"
}

$env:DCA_DB_PATH = Join-Path $runtimeDirectory 'paper.sqlite3'
$env:DCA_BYBIT_TESTNET = '1'
$env:DCA_OKX_DEMO = '1'
$env:RUST_LOG = 'info'
$env:NODE_OPTIONS = '--max-old-space-size=1024'
$env:RAYON_NUM_THREADS = '2'
$env:DCA_LOCAL_RUN = '1'
$env:NEXT_TELEMETRY_DISABLED = '1'
$coreProcess = $null
$uiProcess = $null

function Wait-LocalHttp([string]$Url, [int]$TimeoutSeconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $null = & curl.exe --noproxy '*' --silent --fail --max-time 30 $Url 2>$null
        if ($LASTEXITCODE -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 300
    }
    throw "Local HTTP readiness timed out: $Url. Check logs in $runtimeDirectory."
}

try {
    $coreProcess = Start-Process -FilePath $coreBinary -WorkingDirectory $runtimeDirectory -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $runtimeDirectory 'core.stdout.log') -RedirectStandardError (Join-Path $runtimeDirectory 'core.stderr.log')
    Wait-LocalHttp 'http://127.0.0.1:3002/api/health' 20
    $nodeBinary = (Get-Command node -ErrorAction Stop).Source
    $uiProcess = Start-Process -FilePath $nodeBinary -ArgumentList @('node_modules/next/dist/bin/next', 'dev', '--hostname', '127.0.0.1', '--port', '3000', '--webpack') -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $runtimeDirectory 'ui.stdout.log') -RedirectStandardError (Join-Path $runtimeDirectory 'ui.stderr.log')
    @(
        @{ Id = $coreProcess.Id; Name = $coreProcess.ProcessName; Started = $coreProcess.StartTime.ToUniversalTime().Ticks }
        @{ Id = $uiProcess.Id; Name = $uiProcess.ProcessName; Started = $uiProcess.StartTime.ToUniversalTime().Ticks }
    ) | ConvertTo-Json | Set-Content -LiteralPath $processFile -Encoding UTF8
    Wait-LocalHttp 'http://127.0.0.1:3000/' 120
    Wait-LocalHttp 'http://127.0.0.1:3000/api/health' 20
    Write-Output 'Local terminal: http://127.0.0.1:3000'
    Write-Output "Separate paper database: $env:DCA_DB_PATH"
    Write-Output "Logs: $runtimeDirectory"
} catch {
    foreach ($startedProcess in @($uiProcess, $coreProcess)) {
        if ($null -ne $startedProcess) {
            & taskkill.exe /PID $startedProcess.Id /T /F 2>$null | Out-Null
        }
    }
    Remove-Item -LiteralPath $processFile -ErrorAction SilentlyContinue
    throw
}
