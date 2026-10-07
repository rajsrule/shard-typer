. "$PSScriptRoot\dev-env.ps1"
$taskMetadata = & cargo metadata --locked --offline --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect dependency licenses.' }
$taskTree = & cargo tree --locked --offline --target x86_64-pc-windows-gnu --edges normal --prefix none --format '{p}'
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect Windows dependencies.' }
$taskDependencies = @{}
foreach ($taskLine in $taskTree) {
    if ($taskLine -match '^(\S+) v(\S+)') { $taskDependencies["$($Matches[1])@$($Matches[2])"] = $true }
}
$taskSections = [System.Collections.Generic.List[string]]::new()
$taskSections.Add("SHARD TYPER - THIRD-PARTY NOTICES`r`nSource repositories and licenses are listed below. These dependencies remain under their respective licenses.")
foreach ($taskPackage in ($taskMetadata.packages | Sort-Object name,version)) {
    if (-not $taskPackage.source -or -not $taskDependencies.ContainsKey("$($taskPackage.name)@$($taskPackage.version)")) { continue }
    $taskSections.Add("`r`n$('=' * 70)`r`n$($taskPackage.name) $($taskPackage.version)`r`nLicense: $($taskPackage.license)`r`nSource: $($taskPackage.repository)`r`n")
    $taskDirectory = Split-Path -Parent $taskPackage.manifest_path
    $taskLicenseFiles = Get-ChildItem -LiteralPath $taskDirectory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' }
    foreach ($taskFile in $taskLicenseFiles) { $taskSections.Add((Get-Content -LiteralPath $taskFile.FullName -Raw)) }
    if ($taskPackage.name -eq 'epaint_default_fonts') {
        $taskFontLicenses = Get-ChildItem -LiteralPath $taskDirectory -File -Recurse | Where-Object { $_.Name -match '(OFL|LICENSE|LICENCE|COPYING|NOTICE)' -and $_.DirectoryName -ne $taskDirectory }
        foreach ($taskFile in $taskFontLicenses) { $taskSections.Add((Get-Content -LiteralPath $taskFile.FullName -Raw)) }
    }
}
$taskSections -join "`r`n" | Set-Content -LiteralPath (Join-Path $taskRoot 'docs\THIRD_PARTY_NOTICES.txt') -Encoding utf8

