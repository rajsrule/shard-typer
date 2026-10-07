param([switch]$SkipBuild,[switch]$PortableOnly,[string]$Iscc)
. "$PSScriptRoot\dev-env.ps1"
if (-not $SkipBuild) { & "$PSScriptRoot\build.ps1" }
& "$PSScriptRoot\generate-notices.ps1"
$taskExe = Join-Path $taskRoot 'target\release\ShardTyper.exe'
if (-not (Test-Path -LiteralPath $taskExe)) { throw 'Run scripts/build.ps1 first.' }
$taskManifest = & cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
$taskVersion = $taskManifest.packages[0].version
$taskDist = Join-Path $taskRoot 'dist'
$taskPortable = Join-Path $taskDist "ShardTyper-$taskVersion-portable"
New-Item -ItemType Directory -Force -Path $taskPortable | Out-Null
Copy-Item -LiteralPath $taskExe -Destination (Join-Path $taskPortable 'ShardTyper.exe') -Force
Copy-Item -LiteralPath (Join-Path $taskRoot 'LICENSE') -Destination $taskPortable -Force
Copy-Item -LiteralPath (Join-Path $taskRoot 'docs\QUICKSTART.txt') -Destination $taskPortable -Force
Copy-Item -LiteralPath (Join-Path $taskRoot 'docs\THIRD_PARTY_NOTICES.txt') -Destination $taskPortable -Force
$taskZip = Join-Path $taskDist "ShardTyper-$taskVersion-windows-x64-portable.zip"
Compress-Archive -LiteralPath (Join-Path $taskPortable 'ShardTyper.exe'),(Join-Path $taskPortable 'LICENSE'),(Join-Path $taskPortable 'QUICKSTART.txt'),(Join-Path $taskPortable 'THIRD_PARTY_NOTICES.txt') -DestinationPath $taskZip -Force
if (-not $PortableOnly) {
    if (-not $Iscc) {
        $taskCandidates = @((Join-Path $taskRoot '.build\inno\ISCC.exe'),'C:\Program Files (x86)\Inno Setup 6\ISCC.exe','C:\Program Files\Inno Setup 6\ISCC.exe')
        $Iscc = $taskCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        if (-not $Iscc) { $taskCommand = Get-Command ISCC -ErrorAction SilentlyContinue; if ($taskCommand) { $Iscc = $taskCommand.Source } }
    }
    if (-not $Iscc) { throw 'Portable ZIP is ready. Install Inno Setup to build the installer, or use -PortableOnly.' }
    & $Iscc "/DAppVersion=$taskVersion" "/DSourceRoot=$taskRoot" "/O$taskDist" (Join-Path $taskRoot 'packaging\shard-typer.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer build failed.' }
}
$taskArtifacts = Get-ChildItem -LiteralPath $taskDist -File | Where-Object { $_.Extension -in '.zip','.exe' }
$taskHashes = $taskArtifacts | ForEach-Object { $taskHash = Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256; "$($taskHash.Hash.ToLower())  $($_.Name)" }
$taskHashes | Set-Content -LiteralPath (Join-Path $taskDist 'SHA256SUMS.txt') -Encoding ascii
$taskArtifacts | Select-Object Name,Length

