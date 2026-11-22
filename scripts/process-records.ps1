# Shared process registry helpers for the Windows local-run scripts.
# Normalize legacy/nested ConvertTo-Json output while only accepting records
# with scalar PID, start-time, and process name fields.
function Add-LocalProcessRecordCandidates {
    param(
        [Parameter(Mandatory = $false)] $Value,
        [Parameter(Mandatory = $true)] [AllowEmptyCollection()] [System.Collections.Generic.List[object]] $Records
    )

    if ($null -eq $Value) { return }
    if ($Value -is [System.Array] -or $Value -is [System.Collections.IList]) {
        foreach ($item in $Value) { Add-LocalProcessRecordCandidates -Value $item -Records $Records }
        return
    }

    $properties = $Value.PSObject.Properties
    if ($properties['Id'] -and $properties['Started'] -and $properties['Name']) {
        $id = 0
        $started = 0L
        $name = [string]$Value.Name
        if ([int]::TryParse([string]$Value.Id, [ref]$id) -and $id -gt 0 -and
            [long]::TryParse([string]$Value.Started, [ref]$started) -and $started -gt 0 -and
            $name -in @('dca_server', 'node')) {
            $Records.Add([pscustomobject]@{ Id = $id; Name = $name; Started = $started })
        }
        return
    }

    if ($properties['value']) {
        Add-LocalProcessRecordCandidates -Value $Value.value -Records $Records
    }
}

function Read-LocalProcessRecords {
    param([Parameter(Mandatory = $true)][string] $Path)

    $records = [System.Collections.Generic.List[object]]::new()
    if (-not (Test-Path -LiteralPath $Path)) { return @() }
    try {
        $document = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -ErrorAction Stop
    } catch {
        throw 'Invalid local process record; refusing to stop any process.'
    }
    Add-LocalProcessRecordCandidates -Value $document -Records $records
    $unique = @{}
    foreach ($record in $records) { $unique[[string]$record.Id] = $record }
    return @($unique.Values)
}

function Write-LocalProcessRecords {
    param(
        [Parameter(Mandatory = $true)][string] $Path,
        [Parameter(Mandatory = $false)][object[]] $Records = @()
    )

    $normalized = [System.Collections.Generic.List[object]]::new()
    foreach ($record in $Records) {
        if ($null -ne $record -and $record.Id -and $record.Started -and $record.Name -in @('dca_server', 'node')) {
            $normalized.Add([pscustomobject]@{ Id = [int]$record.Id; Name = [string]$record.Name; Started = [long]$record.Started })
        }
    }
    $payload = ConvertTo-Json -InputObject ([object[]]$normalized.ToArray()) -Depth 3
    [System.IO.File]::WriteAllText($Path, $payload + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
}

function Protect-LocalRuntimeDirectory {
    param([Parameter(Mandatory = $true)][string] $Path)

    $principal = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
    $rule = '{0}:(OI)(CI)F' -f $principal
    & icacls.exe $Path '/inheritance:r' '/grant:r' $rule | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not restrict .local-runtime directory permissions.' }
}

function Protect-LocalSecretFile {
    param([Parameter(Mandatory = $true)][string] $Path)

    $principal = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
    $rule = '{0}:F' -f $principal
    & icacls.exe $Path '/inheritance:r' '/grant:r' $rule | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not restrict local secret file permissions.' }
}

function Get-VerifiedLocalNextProcessPair {
    param(
        [Parameter(Mandatory = $true)][int] $ListenerProcessId,
        [Parameter(Mandatory = $false)][object[]] $Records = @(),
        [Parameter(Mandatory = $true)][string] $ProjectRoot
    )

    $childInfo = Get-CimInstance Win32_Process -Filter ("ProcessId={0}" -f $ListenerProcessId) -ErrorAction SilentlyContinue
    $childProcess = Get-Process -Id $ListenerProcessId -ErrorAction SilentlyContinue
    if (-not $childInfo -or -not $childProcess -or $childInfo.Name -ne 'node.exe') { return $null }
    $serverEntry = (Join-Path $ProjectRoot 'node_modules/next/dist/server/lib/start-server.js').Replace('/', '\')
    $childCommandLine = ([string]$childInfo.CommandLine).Replace('/', '\')
    if ($childCommandLine.IndexOf($serverEntry, [System.StringComparison]::OrdinalIgnoreCase) -lt 0) { return $null }

    $parentId = [int]$childInfo.ParentProcessId
    $record = $Records | Where-Object { [int]$_.Id -eq $parentId -and $_.Name -eq 'node' } | Select-Object -First 1
    $parentInfo = Get-CimInstance Win32_Process -Filter ("ProcessId={0}" -f $parentId) -ErrorAction SilentlyContinue
    $parentProcess = Get-Process -Id $parentId -ErrorAction SilentlyContinue
    if (-not $record -or -not $parentInfo -or -not $parentProcess -or $parentInfo.Name -ne 'node.exe') { return $null }
    try {
        if ([long]$record.Started -ne [long]$parentProcess.StartTime.ToUniversalTime().Ticks) { return $null }
    } catch {
        return $null
    }

    $parentCommandLine = ([string]$parentInfo.CommandLine).Replace('/', '\')
    if ($parentCommandLine.IndexOf('node_modules\next\dist\bin\next dev ', [System.StringComparison]::OrdinalIgnoreCase) -lt 0 -or
        $parentCommandLine.IndexOf('--hostname 127.0.0.1', [System.StringComparison]::OrdinalIgnoreCase) -lt 0 -or
        $parentCommandLine.IndexOf('--port 3000', [System.StringComparison]::OrdinalIgnoreCase) -lt 0) {
        return $null
    }
    return [pscustomobject]@{ ParentId = $parentId; ChildId = $ListenerProcessId }
}
