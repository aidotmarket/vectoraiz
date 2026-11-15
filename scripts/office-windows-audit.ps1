param([ValidateSet('setup','cursor','capture','host','restore')][string]$Mode,
      [string]$State = '', [long]$After = 0, [string]$Nonce = '', [string]$Target = 'mmc.exe')
$ErrorActionPreference = 'Stop'

function Convert-AuditBackupReadback([string[]]$Csv) {
    # /get /r has only six columns. /backup adds numeric Setting Value.
    # Use documented column positions: headings and display values can localize.
    $rows = @($Csv | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | ConvertFrom-Csv)
    $matches = @($rows | Where-Object {
        $fields = @($_.PSObject.Properties)
        $fields.Count -eq 7 -and $fields[3].Value.Trim() -ieq '{0CCE922B-69AE-11D9-BED3-505054503030}'
    })
    if ($matches.Count -ne 1) { throw 'auditpol backup readback must contain one Process Creation subcategory' }
    $fields = @($matches[0].PSObject.Properties)
    switch ($fields[6].Value.Trim()) {
        '1' { 'success' }
        '3' { 'success-and-failure' }
        default { throw 'Success auditing not verified by numeric backup readback' }
    }
}

try {
    switch ($Mode) {
        setup {
            $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
            $principal = [Security.Principal.WindowsPrincipal]::new($identity)
            if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
                @{success=$false; elevation_required=$true; error='Security 4688 audit setup requires elevated PowerShell'; reason='Security 4688 audit setup requires elevated PowerShell'} | ConvertTo-Json -Compress
                exit 0
            }
            if (-not $State -or (Test-Path $State) -or (Test-Path "$State.csv")) { throw 'fresh audit state path required' }
            $key = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Policies\System\Audit'
            $property = 'ProcessCreationIncludeCmdLine_Enabled'
            $keyExisted = Test-Path $key
            $item = if ($keyExisted) { Get-Item $key } else { $null }
            $valueExisted = $item -and ($item.GetValueNames() -contains $property)
            $saved = @{key_existed=[bool]$keyExisted; value_existed=[bool]$valueExisted;
                       value=$(if ($valueExisted) { $item.GetValue($property) } else { $null });
                       kind=$(if ($valueExisted) { [string]$item.GetValueKind($property) } else { $null })}
            & auditpol /backup /file:"$State.csv" | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'audit policy backup failed; no settings changed' }
            $saved | Export-Clixml -LiteralPath $State
            # Process Creation subcategory GUID avoids localized auditpol names.
            $output = & auditpol /set /subcategory:'{0CCE922B-69AE-11D9-BED3-505054503030}' /success:enable 2>&1
            if ($LASTEXITCODE -ne 0) { throw 'auditpol set failed' }
            $key = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Policies\System\Audit'
            New-Item $key -Force | Out-Null
            New-ItemProperty $key -Name ProcessCreationIncludeCmdLine_Enabled -Value 1 -PropertyType DWord -Force | Out-Null
            $verification = "$State.$([guid]::NewGuid().ToString('N')).verify.csv"
            if (Test-Path -LiteralPath $verification) { throw 'fresh audit verification path required' }
            try {
                & auditpol /backup /file:"$verification" | Out-Null
                if ($LASTEXITCODE -ne 0) { throw 'auditpol verification backup failed' }
                $inclusion = Convert-AuditBackupReadback (Get-Content -LiteralPath $verification)
            } finally {
                if (Test-Path -LiteralPath $verification) { Remove-Item -LiteralPath $verification }
            }
            @{success=$true; inclusion_setting=$inclusion; command_line_auditing=(Get-ItemProperty $key).ProcessCreationIncludeCmdLine_Enabled} | ConvertTo-Json -Compress
        }
        restore {
            if (-not $State -or -not (Test-Path $State)) { throw 'saved audit state required' }
            $saved = Import-Clixml -LiteralPath $State
            & auditpol /restore /file:"$State.csv" | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'audit policy restore failed; saved state retained' }
            $key = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Policies\System\Audit'
            $property = 'ProcessCreationIncludeCmdLine_Enabled'
            if ($saved.value_existed) {
                New-ItemProperty $key -Name $property -Value $saved.value -PropertyType $saved.kind -Force | Out-Null
            } elseif ((Test-Path $key) -and ((Get-Item $key).GetValueNames() -contains $property)) {
                Remove-ItemProperty $key -Name $property
            }
            if (-not $saved.key_existed -and (Test-Path $key)) {
                $item = Get-Item $key
                if ($item.ValueCount -eq 0 -and $item.SubKeyCount -eq 0) { Remove-Item $key }
            }
            @{success=$true; restored=$true} | ConvertTo-Json -Compress
        }
        cursor {
            $event = Get-WinEvent -LogName Security -MaxEvents 1
            @{success=$true; record_id=$event.RecordId} | ConvertTo-Json -Compress
        }
        capture {
            # Persist full XML privately. Python publishes only matched counts/IDs.
            $latest = Get-WinEvent -LogName Security -MaxEvents 1
            $queryErrors = @()
            $events = @(Get-WinEvent -LogName Security -FilterXPath "*[System[(EventID=4688) and (EventRecordID > $After) and (EventRecordID <= $($latest.RecordId))]]" -ErrorAction Continue -ErrorVariable queryErrors 2>$null)
            foreach ($queryError in $queryErrors) {
                if ($queryError.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound,*') { throw $queryError }
            }
            $all = @($events | ForEach-Object { $_.ToXml() })
            $nonceIds = @(); $targetIds = @()
            foreach ($event in $events) {
                [xml]$xml = $event.ToXml()
                $data = @{}
                foreach ($field in $xml.Event.EventData.Data) { $data[$field.Name] = [string]$field.'#text' }
                if ($Nonce -and $data.CommandLine -and $data.CommandLine.Contains($Nonce)) { $nonceIds += $event.RecordId }
                if ($data.NewProcessName -and [IO.Path]::GetFileName($data.NewProcessName) -ieq $Target) { $targetIds += $event.RecordId }
            }
            @{success=$true; filtered_query_success=$true; after=$After; through=$latest.RecordId; xml=$all;
              nonce_record_ids=$nonceIds; target_record_ids=$targetIds;
              target_running=[bool](Get-Process ([IO.Path]::GetFileNameWithoutExtension($Target)) -ErrorAction SilentlyContinue)} | ConvertTo-Json -Depth 5 -Compress
        }
        host {
            $os = Get-CimInstance Win32_OperatingSystem
            $featuresAvailable = $true
            try { $features = @(Get-WindowsOptionalFeature -Online | Where-Object { $_.FeatureName -match 'Hyper-V|VirtualMachinePlatform|Subsystem-Linux|Containers-DisposableClientVM' } | Select-Object FeatureName,State) } catch { $features = @(); $featuresAvailable = $false }
            $vms = @(); if (Get-Command Get-VM -ErrorAction SilentlyContinue) { $vms = @(); try { $vms = @(Get-VM | Select-Object State) } catch { $featuresAvailable = $false } }
            @{success=$true; product_type=$os.ProductType; build=$os.BuildNumber; edition=$os.Caption; interactive_session=([Environment]::UserInteractive -and (Get-Process -Id $PID).SessionId -gt 0); features=$features; features_available=$featuresAvailable;
              utility_vms=$vms; target_running=[bool](Get-Process mmc -ErrorAction SilentlyContinue);
              vectoraiz_running=[bool](Get-Process vectoraiz -ErrorAction SilentlyContinue)} | ConvertTo-Json -Depth 5 -Compress
        }
    }
} catch {
    @{success=$false; filtered_query_success=$false; error=$_.Exception.Message} | ConvertTo-Json -Compress
    exit 1
}
