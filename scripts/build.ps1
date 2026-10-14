<#
.SYNOPSIS
    Builds termnest for Windows inside Docker, or runs the test suite.

.EXAMPLE
    ./scripts/build.ps1          # release build -> release/termnest.exe and the NSIS installer
    ./scripts/build.ps1 -Test    # type check, lint, Rust tests and clippy
#>
param(
    [switch]$Test
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

docker info --format '{{.ServerVersion}}' *> $null
if ($LASTEXITCODE -ne 0) {
    Write-Error 'Docker is not running. Start Docker Desktop and try again.'
    exit 1
}

$service = if ($Test) { 'test' } else { 'build' }
docker compose run --rm --build $service
exit $LASTEXITCODE
