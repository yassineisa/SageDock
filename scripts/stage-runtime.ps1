param([string]$Archive = (Join-Path $env:USERPROFILE 'SageDock-Runtime/sagedock-runtime-sage10.9-x64.tar.xz'))
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$source = (Resolve-Path -LiteralPath $Archive).Path
$manifest = Get-Content -Raw -LiteralPath "$source.json" | ConvertFrom-Json
$trusted = Get-Content -Raw -LiteralPath (Join-Path $repo 'src-tauri/trusted-runtimes.json') | ConvertFrom-Json
$approved = @($trusted | Where-Object { $_.sha256 -eq $manifest.sha256 -and $_.size -eq $manifest.size -and $_.arch -eq $manifest.arch })
if ($approved.Count -eq 0) { throw 'This runtime is not pinned in trusted-runtimes.json. Review and approve the release artifact first.' }
if ((Get-Item -LiteralPath $source).Length -ne $manifest.size) { throw 'Runtime size mismatch.' }
Write-Host 'Verifying the runtime archive...'
if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $manifest.sha256) { throw 'Runtime checksum mismatch.' }
$destination = Join-Path $repo 'src-tauri/runtime'
New-Item -ItemType Directory -Path $destination -Force | Out-Null
Copy-Item -LiteralPath $source -Destination (Join-Path $destination ([IO.Path]::GetFileName($source))) -Force
Copy-Item -LiteralPath "$source.json" -Destination (Join-Path $destination ([IO.Path]::GetFileName("$source.json"))) -Force
Write-Host 'Verified runtime staged. Build the desktop installer with npm.cmd run tauri build.'
