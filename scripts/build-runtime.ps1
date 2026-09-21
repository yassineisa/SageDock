<#
.SYNOPSIS
    Builds the SageDock runtime image: Ubuntu + SageMath (from conda-forge) + JupyterLab,
    packaged as a WSL-importable archive plus a manifest the SageDock app verifies.

.DESCRIPTION
    This is a release step, run once per SageMath version by whoever ships SageDock. It is
    never run on an end user's machine. SageMath stopped publishing prebuilt Linux binaries,
    and its large downloads are unreliable, so the slow network-dependent work happens here,
    with retries, and students only ever import the finished file.

    Output (default: %USERPROFILE%\SageDock-Runtime):
        sagedock-runtime-sage<version>-<arch>.tar.xz
        sagedock-runtime-sage<version>-<arch>.tar.xz.json   (checksum manifest; ship both together)
        build.log

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build-runtime.ps1
#>
param(
    [string]$SageVersion = '10.9',
    [string]$MiniforgeVersion = '26.7.2-0',
    # Deliberately NOT inside the repository: it lives under a OneDrive-synced Desktop here,
    # and a multi-gigabyte archive there would be uploaded to the cloud.
    [string]$OutDir = (Join-Path $env:USERPROFILE 'SageDock-Runtime'),
    [ValidateSet('tar.xz', 'tar.gz')]
    [string]$Format = 'tar.xz'
)

# 'Continue', not 'Stop': in Windows PowerShell 5.1 any stderr line from a native program
# becomes an error record, and 'Stop' would abort a healthy build on the first warning.
# Every native call's exit code is checked explicitly instead.
$ErrorActionPreference = 'Continue'

$BuildDistro = 'SageDockBuild'
$WorkDir = Join-Path $env:LOCALAPPDATA 'SageDockBuild'

$BaseImages = @{
    'AMD64' = @{
        Url    = 'https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release/ubuntu-base-24.04.5-base-amd64.tar.gz'
        Sha256 = 'e77b6f10c2590cef872b33ee9f635a0e3fd1f57fb074c0e52b5c7f56147a0c86'
        Arch   = 'x64'
    }
    'ARM64' = @{
        Url    = 'https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release/ubuntu-base-24.04.5-base-arm64.tar.gz'
        Sha256 = 'a91d5a93010193712d346d761372b7c9db6dfcf093893161c64ca107f05914f2'
        Arch   = 'arm64'
    }
}

function Write-Step($message) {
    Write-Host ''
    Write-Host "==> $message" -ForegroundColor Cyan
}

function Assert-ExitCode($what) {
    if ($LASTEXITCODE -ne 0) { throw "$what failed with exit code $LASTEXITCODE" }
}

function Test-DistroRegistered($name) {
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss'
    if (-not (Test-Path $key)) { return $false }
    foreach ($child in Get-ChildItem $key) {
        if ((Get-ItemProperty $child.PSPath).DistributionName -eq $name) { return $true }
    }
    return $false
}

function ConvertTo-WslPath($windowsPath) {
    $full = [IO.Path]::GetFullPath($windowsPath)
    $drive = $full.Substring(0, 1).ToLower()
    return "/mnt/$drive" + ($full.Substring(2) -replace '\\', '/')
}

function Write-Utf8NoBom($path, $text) {
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding($false)))
}

New-Item -ItemType Directory -Force $OutDir, $WorkDir | Out-Null
Start-Transcript -Path (Join-Path $OutDir 'build.log') -Force | Out-Null
$started = Get-Date

try {
    Write-Step 'Checking prerequisites'
    & wsl.exe --status *> $null
    if ($LASTEXITCODE -ne 0) { throw 'WSL is not working on this machine. Install it and restart first.' }

    $base = $BaseImages[$env:PROCESSOR_ARCHITECTURE]
    if (-not $base) { throw "Unsupported architecture: $env:PROCESSOR_ARCHITECTURE" }

    $archiveName = "sagedock-runtime-sage$SageVersion-$($base.Arch).$Format"
    $archivePath = Join-Path $OutDir $archiveName
    $manifestPath = "$archivePath.json"

    # The build distro name is owned by this script, so a leftover one can only be the
    # remains of an earlier failed build.
    if (Test-DistroRegistered $BuildDistro) {
        Write-Host "Removing leftover build environment '$BuildDistro' from a previous run"
        & wsl.exe --unregister $BuildDistro | Out-Null
    }

    Write-Step 'Downloading Ubuntu base image'
    $baseTar = Join-Path $WorkDir 'ubuntu-base.tar.gz'
    $ok = $false
    for ($attempt = 1; $attempt -le 5 -and -not $ok; $attempt++) {
        if (-not (Test-Path $baseTar)) {
            try {
                Invoke-WebRequest -Uri $base.Url -OutFile $baseTar -UseBasicParsing
            } catch {
                Write-Host "  download attempt $attempt failed: $($_.Exception.Message)"
                Start-Sleep -Seconds (10 * $attempt)
                continue
            }
        }
        $actual = (Get-FileHash $baseTar -Algorithm SHA256).Hash.ToLower()
        if ($actual -eq $base.Sha256) {
            $ok = $true
        } else {
            Write-Host "  checksum mismatch on attempt $attempt; discarding and re-downloading"
            Remove-Item $baseTar -Force
        }
    }
    if (-not $ok) { throw 'Could not download a verified Ubuntu base image.' }

    Write-Step 'Creating build environment'
    & wsl.exe --import $BuildDistro (Join-Path $WorkDir 'disk') $baseTar --version 2
    Assert-ExitCode 'wsl --import'

    Write-Step "Installing SageMath $SageVersion (this takes a while)"
    # Copied with LF endings and no BOM: bash rejects both CRLF and a leading byte-order mark.
    $scriptSource = Join-Path $PSScriptRoot 'runtime\provision.sh'
    $scriptCopy = Join-Path $WorkDir 'provision.sh'
    Write-Utf8NoBom $scriptCopy ((Get-Content -Raw $scriptSource) -replace "`r`n", "`n")

    # --exec runs bash directly with discrete arguments, never through a shell that could
    # reinterpret them.
    & wsl.exe -d $BuildDistro -u root --exec bash (ConvertTo-WslPath $scriptCopy) $SageVersion $MiniforgeVersion
    Assert-ExitCode 'provisioning'

    Write-Step 'Verifying SageMath works'
    # wsl.conf only applies from the next start, so restart before testing as the real user.
    & wsl.exe --terminate $BuildDistro | Out-Null

    $verify = (& wsl.exe -d $BuildDistro -u sage --exec /opt/sagedock/bin/sagedock-verify 2>&1) -join "`n"
    Write-Host $verify
    if ($verify -notmatch [regex]::Escape('2^6 * 3 * 643')) { throw 'SageMath did not compute factor(123456) correctly.' }
    if ($verify -notmatch 'PYTHON_IMPORTS=ok') { throw 'The scientific Python packages did not import.' }
    if ($verify -notmatch 'sagemath') { throw 'No Jupyter kernel named "sagemath" was found.' }

    $selftest = (& wsl.exe -d $BuildDistro -u sage --exec /opt/sagedock/bin/sagedock-selftest 2>&1) -join "`n"
    Write-Host $selftest
    if ($selftest -notmatch 'KERNEL_OK') { throw 'A notebook cell could not be executed through the SageMath kernel.' }

    Write-Step "Exporting image ($Format)"
    & wsl.exe --terminate $BuildDistro | Out-Null
    if (Test-Path $archivePath) { Remove-Item $archivePath -Force }
    & wsl.exe --export $BuildDistro $archivePath --format $Format
    Assert-ExitCode 'wsl --export'

    Write-Step 'Writing checksum manifest'
    $file = Get-Item $archivePath
    $manifest = [ordered]@{
        format       = 1
        sage_version = $SageVersion
        arch         = $base.Arch
        archive      = $file.Name
        size         = $file.Length
        sha256       = (Get-FileHash $archivePath -Algorithm SHA256).Hash.ToLower()
        built_utc    = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    }
    Write-Utf8NoBom $manifestPath ($manifest | ConvertTo-Json)

    Write-Step 'Cleaning up build environment'
    & wsl.exe --unregister $BuildDistro | Out-Null
    Remove-Item -Recurse -Force $WorkDir -ErrorAction SilentlyContinue

    $sizeGb = [math]::Round($file.Length / 1GB, 2)
    $elapsed = [math]::Round(((Get-Date) - $started).TotalMinutes, 1)
    Write-Host ''
    Write-Host "BUILD_OK  $sizeGb GB in $elapsed min" -ForegroundColor Green
    Write-Host "  $archivePath"
    Write-Host "  $manifestPath"
    if ($file.Length -ge 2GB) {
        Write-Host ''
        Write-Host 'Note: this file is over 2 GB. MSI and NSIS installers cannot embed files that large;' -ForegroundColor Yellow
        Write-Host 'ship it alongside the installer instead (SageDock will find it or ask for it).' -ForegroundColor Yellow
    }
} catch {
    Write-Host ''
    Write-Host "BUILD_FAILED: $($_.Exception.Message)" -ForegroundColor Red
    if (Test-DistroRegistered $BuildDistro) {
        Write-Host "The build environment was kept for inspection. Open it with:  wsl -d $BuildDistro"
        Write-Host "Remove it with:  wsl --unregister $BuildDistro"
    }
    exit 1
} finally {
    Stop-Transcript | Out-Null
}
