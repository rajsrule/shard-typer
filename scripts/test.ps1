. "$PSScriptRoot\dev-env.ps1"
& cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed.' }
& cargo clippy --locked --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Lint check failed.' }
& cargo test --locked
if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }

