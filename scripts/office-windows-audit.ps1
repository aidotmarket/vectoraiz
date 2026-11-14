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
            @{success=$true; policy=@($policy); command_line_auditing=(Get-ItemProperty $key).ProcessCreationIncludeCmdLine_Enabled} | ConvertTo-Json -Compress
        }
        cursor {
            $event = Get-WinEvent -LogName Security -MaxEvents 1
            @{success=$true; record_id=$event.RecordId} | ConvertTo-Json -Compress
        }
        capture {
            # Persist full XML privately. Python publishes only matched counts/IDs.
            $events = @(Get-WinEvent -LogName Security -FilterXPath "*[System[(EventID=4688) and (EventRecordID > $After)]]" -ErrorAction SilentlyContinue)
            $all = @($events | ForEach-Object { $_.ToXml() })
            $nonceIds = @(); $targetIds = @()
            foreach ($event in $events) {
                [xml]$xml = $event.ToXml()
                $data = @{}
                foreach ($field in $xml.Event.EventData.Data) { $data[$field.Name] = [string]$field.'#text' }
                if ($Nonce -and $data.CommandLine -and $data.CommandLine.Contains($Nonce)) { $nonceIds += $event.RecordId }
                if ($data.NewProcessName -and [IO.Path]::GetFileName($data.NewProcessName) -ieq $Target) { $targetIds += $event.RecordId }
            }
            # Reading the channel must succeed even when the window has no 4688s.
            $latest = Get-WinEvent -LogName Security -MaxEvents 1
            @{success=$true; after=$After; through=$latest.RecordId; xml=$all;
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
    @{success=$false; error=$_.Exception.Message} | ConvertTo-Json -Compress
    exit 1
}
