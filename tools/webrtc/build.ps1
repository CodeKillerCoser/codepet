$ErrorActionPreference = 'Stop'
& node (Join-Path $PSScriptRoot 'build.mjs') @args
exit $LASTEXITCODE
