param([ValidateSet('setup','cursor','capture','host')][string]$Mode,
      [long]$After = 0, [string]$Nonce = '', [string]$Target = 'mmc.exe')
$ErrorActionPreference = 'Stop'
try {
    switch ($Mode) {
        setup {
            # Process Creation subcategory GUID avoids localized auditpol names.
            $output = & auditpol /set /subcategory:'{0CCE922B-69AE-11D9-BED3-505054503030}' /success:enable 2>&1
            if ($LASTEXITCODE -ne 0) { throw "auditpol failed: $output" }
            $key = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Policies\System\Audit'
            New-Item $key -Force | Out-Null
            New-ItemProperty $key -Name ProcessCreationIncludeCmdLine_Enabled -Value 1 -PropertyType DWord -Force | Out-Null
            $policy = & auditpol /get /subcategory:'{0CCE922B-69AE-11D9-BED3-505054503030}' /r
            if ($LASTEXITCODE -ne 0) { throw 'auditpol verification failed' }
            $rows = @($policy | ConvertFrom-Csv)
            if ($rows.Count -ne 1) { throw 'auditpol readback must contain one subcategory' }
            # CSV field order is stable; reject unrecognised/localised values closed.
            $setting = @($rows[0].PSObject.Properties)[4].Value.Trim()
            $inclusion = switch ($setting) {
                'Success' { 'success' }
                'Success and Failure' { 'success-and-failure' }
                default { throw "Success auditing not verified: $setting" }
            }
            @{success=$true; inclusion_setting=$inclusion; policy=@($policy); command_line_auditing=(Get-ItemProperty $key).ProcessCreationIncludeCmdLine_Enabled} | ConvertTo-Json -Compress
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
            $features = @(Get-WindowsOptionalFeature -Online | Where-Object { $_.FeatureName -match 'Hyper-V|VirtualMachinePlatform|Subsystem-Linux|Containers-DisposableClientVM' } | Select-Object FeatureName,State)
            $vms = @(); if (Get-Command Get-VM -ErrorAction SilentlyContinue) { $vms = @(Get-VM | Select-Object State) }
            @{success=$true; product_type=$os.ProductType; build=$os.BuildNumber; features=$features;
              utility_vms=$vms; target_running=[bool](Get-Process mmc -ErrorAction SilentlyContinue);
              vectoraiz_running=[bool](Get-Process vectoraiz -ErrorAction SilentlyContinue)} | ConvertTo-Json -Depth 5 -Compress
        }
    }
} catch {
    @{success=$false; filtered_query_success=$false; error=$_.Exception.Message} | ConvertTo-Json -Compress
    exit 1
}
