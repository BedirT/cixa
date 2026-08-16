$ErrorActionPreference = "Stop"
& "$PSScriptRoot\verify"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
