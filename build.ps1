$ErrorActionPreference = 'Stop'
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    throw 'MultiCAD build needs Node.js 22.14+ for developer tooling. Install Node.js, then rerun.'
}
& node (Join-Path $PSScriptRoot 'tools/build.mjs') @args
exit $LASTEXITCODE
