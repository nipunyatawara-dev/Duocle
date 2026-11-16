param([string]$Target = 'x86_64-pc-windows-msvc')
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    if ($Target -ne 'x86_64-pc-windows-msvc') { throw 'The installer currently targets x64 Windows.' }
    cargo build --release --locked --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed.' }
    New-Item -ItemType Directory -Force dist | Out-Null
    makensis packaging/windows.nsi
    if ($LASTEXITCODE -ne 0) { throw 'NSIS packaging failed.' }
} finally {
    Pop-Location
}
