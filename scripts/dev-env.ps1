$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskCargoHome = Join-Path $taskRoot '.build\cargo'
if (Test-Path -LiteralPath (Join-Path $taskCargoHome 'bin\cargo.exe')) {
    $env:CARGO_HOME = $taskCargoHome
    $env:RUSTUP_HOME = Join-Path $taskRoot '.build\rustup'
    $taskNativeTools = Join-Path $taskRoot '.build\tools\ucrt64\bin'
    $taskGnuTools = Join-Path $env:RUSTUP_HOME 'toolchains\1.99.0-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained'
    $env:PATH = "$taskCargoHome\bin;$taskNativeTools;$taskGnuTools;$env:PATH"
    if (Test-Path -LiteralPath (Join-Path $taskNativeTools 'windres.exe')) { $env:RC = Join-Path $taskNativeTools 'windres.exe' }
    if (Test-Path -LiteralPath (Join-Path $taskGnuTools 'x86_64-w64-mingw32-gcc.exe')) { $env:CC = Join-Path $taskGnuTools 'x86_64-w64-mingw32-gcc.exe' }
    if (Test-Path -LiteralPath (Join-Path $taskGnuTools 'x86_64-w64-mingw32-gcc.exe')) { $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = Join-Path $taskGnuTools 'x86_64-w64-mingw32-gcc.exe' }
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Install Rust and Windows build tools first. See README.md for the one-time setup.'
}
Set-Location -LiteralPath $taskRoot

