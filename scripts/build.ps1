param([switch]$Debug)
. "$PSScriptRoot\dev-env.ps1"
$taskArguments = @('build','--locked')
if (-not $Debug) { $taskArguments += '--release' }
& cargo @taskArguments
if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
Write-Output ('Built ' + (Join-Path $taskRoot ('target\' + $(if ($Debug) { 'debug' } else { 'release' }) + '\ShardTyper.exe')))

