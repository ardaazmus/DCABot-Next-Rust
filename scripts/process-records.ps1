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
