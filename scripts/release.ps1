<#
.SYNOPSIS
    Builds and signs a SageDock release with Microsoft Azure Artifact Signing.

.DESCRIPTION
    Enforces this order, which is the only correct one for a Tauri/WiX build:

        build binaries -> sign binaries -> verify -> build MSI from signed binaries
                       -> sign MSI -> verify MSI

    Signing is delegated to Tauri's `bundle.windows.signCommand` rather than applied by
    hand between steps. That is not a convenience: `tauri build` patches the freshly
    linked executable with bundle metadata *after* the linker runs and *before* WiX
    packages it. Signing the binary ourselves beforehand would produce an installer full
    of binaries whose signatures had been invalidated by that patch. Tauri invokes the
    sign command after patching and before packaging, and again on the finished MSI, so
    the required order is preserved and every payload binary is signed in place.

    The script fails the release if any signing or verification step fails.

    No credential is handled here. Azure.CodeSigning.Dlib.dll authenticates with
    DefaultAzureCredential, which picks up the existing `az login` session. Nothing in
    this script reads, writes, prints or stores a token, secret or private key.
    See build/signing/README.md.

.PARAMETER SkipTests
    Skip the validation gate. For iterating on the signing step only — never for a
    release that will actually be published.
#>
[CmdletBinding()]
param(
    [switch]$SkipTests
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot
$ExpectedSubscription = 'f2fd1696-0b89-45a3-946f-e4d211585768'
$TimestampUrl = 'http://timestamp.acs.microsoft.com'
$SigningClientVersion = '1.0.95'

function Write-Step { param([string]$Text) Write-Host "`n=== $Text ===" -ForegroundColor Cyan }
function Fail { param([string]$Text) Write-Host "RELEASE FAILED: $Text" -ForegroundColor Red; exit 1 }

# --- locate tooling ---------------------------------------------------------------------

function Resolve-Az {
    $cmd = Get-Command az.cmd -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $candidates = @(
        "$env:ProgramFiles\Microsoft SDKs\Azure\CLI2\wbin\az.cmd",
        "${env:ProgramFiles(x86)}\Microsoft SDKs\Azure\CLI2\wbin\az.cmd"
    )
    foreach ($c in $candidates) { if (Test-Path $c) { return $c } }
    Fail 'Azure CLI (az) was not found. Install it and run `az login`.'
}

function Resolve-SignTool {
    # Newest x64 SignTool from the installed Windows SDKs. x64 is required: the DLIB must
    # match SignTool's architecture.
    $found = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -like '*\x64\*' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $found) { Fail 'x64 signtool.exe was not found. Install the Windows SDK signing tools.' }
    return $found.FullName
}

function Resolve-Dlib {
    # The signing client is a build tool, not source: it lives in git-ignored .signing/
    # and is fetched on demand.
    $dlib = Join-Path $RepoRoot '.signing\client\bin\x64\Azure.CodeSigning.Dlib.dll'
    if (Test-Path $dlib) { return $dlib }

    Write-Host 'Azure.CodeSigning.Dlib.dll not present; fetching Microsoft.Trusted.Signing.Client...'
    $dir = Join-Path $RepoRoot '.signing'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $zip = Join-Path $dir 'client.zip'
    $url = "https://api.nuget.org/v3-flatcontainer/microsoft.trusted.signing.client/$SigningClientVersion/microsoft.trusted.signing.client.$SigningClientVersion.nupkg"
    Invoke-WebRequest -Uri $url -OutFile $zip -TimeoutSec 600
    $extract = Join-Path $dir 'client'
    if (Test-Path $extract) { Remove-Item $extract -Recurse -Force }
    Expand-Archive -Path $zip -DestinationPath $extract -Force
    Remove-Item $zip -Force
    if (-not (Test-Path $dlib)) { Fail 'The signing client was downloaded but the x64 DLIB is missing.' }
    return $dlib
}

# --- preflight --------------------------------------------------------------------------

Write-Step 'Azure CLI session'
$az = Resolve-Az
# Prints a name, a subscription id and a user name. Deliberately not `get-access-token`:
# nothing in this release process materialises a raw token.
& $az account show --query "{Name:name, ID:id, User:user.name}" -o table
if ($LASTEXITCODE -ne 0) { Fail 'Not signed in to Azure. Run `az login` first.' }

# The DLIB authenticates with AzureCliCredential, which shells out to `az`. On a machine
# where the Azure CLI is installed but not on PATH, that lookup fails and the sign falls
# through the credential chain instead of using the session that is plainly there.
$azDir = Split-Path -Parent $az
if ($env:PATH -notlike "*$azDir*") { $env:PATH = "$azDir;$env:PATH" }

$activeId = (& $az account show --query id -o tsv).Trim()
if ($activeId -ne $ExpectedSubscription) {
    Write-Host "Active subscription is $activeId; switching to $ExpectedSubscription"
    & $az account set --subscription $ExpectedSubscription
    if ($LASTEXITCODE -ne 0) { Fail "Could not select subscription $ExpectedSubscription." }
    $activeId = (& $az account show --query id -o tsv).Trim()
}
if ($activeId -ne $ExpectedSubscription) { Fail "Expected subscription $ExpectedSubscription, got $activeId." }
Write-Host "Subscription confirmed: $activeId" -ForegroundColor Green

Write-Step 'Signing tooling'
$signtool = Resolve-SignTool
$dlib = Resolve-Dlib
$metadata = Join-Path $RepoRoot 'build\signing\signing-metadata.json'
if (-not (Test-Path $metadata)) { Fail "Signing metadata not found at $metadata" }

# A release must never be signed with a metadata file that has acquired a secret.
$metaText = Get-Content $metadata -Raw
foreach ($banned in @('AccessToken', 'access_token', 'RefreshToken', 'ClientSecret', 'client_secret', 'PrivateKey', 'BEGIN ')) {
    if ($metaText -match [regex]::Escape($banned)) {
        Fail "signing-metadata.json appears to contain credential material ('$banned'). Refusing to continue."
    }
}
Write-Host "SignTool: $signtool"
Write-Host "DLIB:     $dlib"
Write-Host "Metadata: $metadata (verified free of credential material)"

# --- validation gate --------------------------------------------------------------------

if (-not $SkipTests) {
    Write-Step 'Validation gate'
    Push-Location $RepoRoot
    try {
        foreach ($step in @('format:check', 'format:rust:check', 'typecheck:tools', 'lint', 'lint:rust', 'test:rust')) {
            Write-Host "npm run $step"
            & npm run $step
            if ($LASTEXITCODE -ne 0) { Fail "Validation step '$step' failed." }
        }
    } finally { Pop-Location }
}

# --- build and sign ---------------------------------------------------------------------

# Tauri replaces %1 with each file it wants signed: first the patched executable, then the
# finished MSI. /v gives per-file output; a non-zero exit fails the bundle, which fails
# the release.
$signArgs = @(
    'sign',
    '/v',
    '/fd', 'SHA256',
    '/tr', $TimestampUrl,
    '/td', 'SHA256',
    '/dlib', $dlib,
    '/dmdf', $metadata,
    '%1'
)
# Written to a file rather than passed as a --config JSON string: the string form has to
# survive PowerShell, npx and node quoting on Windows, and a mangled overlay would silently
# produce an *unsigned* release rather than an error.
#
# Generated at build time and git-ignored, because it embeds machine-specific absolute
# paths to SignTool and the DLIB. The committed configuration stays clean, and signing
# stays opt-in to this script rather than something a plain `tauri build` half-attempts.
$overlayPath = Join-Path $RepoRoot '.signing\tauri.signing.json'
@{
    bundle = @{
        windows = @{
            signCommand = @{
                cmd  = $signtool
                args = $signArgs
            }
        }
    }
} | ConvertTo-Json -Depth 10 | Set-Content -Path $overlayPath -Encoding utf8

Write-Step 'Build, sign binaries, package MSI, sign MSI'
Push-Location $RepoRoot
try {
    # A running SageDock holds the linker lock, and Windows Defender transiently locks a
    # freshly linked binary. Both produce confusing failures; clearing the exe up front
    # avoids the common one.
    Get-Process sagedock -ErrorAction SilentlyContinue | ForEach-Object {
        Fail 'SageDock is running. Close it before building a release.'
    }
    $staleExe = Join-Path $RepoRoot 'src-tauri\target\release\sagedock.exe'
    if (Test-Path $staleExe) { Remove-Item $staleExe -Force -ErrorAction SilentlyContinue }

    & npx tauri build --config $overlayPath
    if ($LASTEXITCODE -ne 0) { Fail 'tauri build failed (this includes a failure of the sign command).' }
} finally { Pop-Location }

# --- verify ------------------------------------------------------------------------------

function Assert-Signed {
    param([string]$Path, [string]$Label)
    Write-Step "Verify signature: $Label"
    if (-not (Test-Path $Path)) { Fail "$Label was not produced at $Path" }
    & $signtool verify /pa /v $Path
    if ($LASTEXITCODE -ne 0) { Fail "$Label failed signature verification." }
    Write-Host "$Label signature verified." -ForegroundColor Green
}

$msi = Get-ChildItem (Join-Path $RepoRoot 'src-tauri\target\release\bundle\msi') -Filter '*.msi' -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $msi) { Fail 'No MSI was produced.' }

Assert-Signed -Path $msi.FullName -Label $msi.Name

# Verify the binaries that actually ship, by extracting them back out of the MSI.
#
# NOT src-tauri\target\release\sagedock.exe. Tauri patches the linked executable with
# bundle metadata, signs the patched copy, harvests that into the MSI, and then restores
# the original bytes on disk — so the file left in target\release is unpatched, unsigned,
# and is not what anybody installs. Verifying it reports "No signature found" on a release
# whose payload is correctly signed, and, far worse, verifying it *successfully* would
# prove nothing about the installer. The shipped copy is the only honest subject.
Write-Step 'Verify binaries inside the MSI'
$extract = Join-Path ([System.IO.Path]::GetTempPath()) ("sagedock-verify-" + [guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Force -Path $extract | Out-Null
    # An administrative install unpacks the payload without touching the installed system.
    $p = Start-Process msiexec.exe -ArgumentList @('/a', "`"$($msi.FullName)`"", '/qn', "TARGETDIR=`"$extract`"") -Wait -PassThru
    if ($p.ExitCode -ne 0) { Fail "Could not extract the MSI payload for verification (msiexec exit $($p.ExitCode))." }

    # Every Authenticode-signable payload file, wherever it sits in the layout. The MSI
    # copy that lands beside the payload is the stub, not the signed installer, so it is
    # excluded — the real one was verified above.
    # @() is load-bearing, not style. This MSI ships exactly one signable PE, and a
    # single-item Get-ChildItem result is a bare FileInfo, which has no .Count — under
    # Set-StrictMode that threw *after* every signature had already verified, failing a
    # perfectly good release. Forcing an array keeps both the emptiness test and the count
    # honest however many binaries the payload grows to.
    $payload = @(
        Get-ChildItem $extract -Recurse -File -Include *.exe, *.dll |
            Where-Object { $_.FullName -ne (Join-Path $extract $msi.Name) }
    )
    if ($payload.Count -eq 0) { Fail 'No EXE or DLL was found inside the MSI; expected at least sagedock.exe.' }

    foreach ($file in $payload) {
        Assert-Signed -Path $file.FullName -Label "in MSI: $($file.Name)"
    }
    Write-Host "All $($payload.Count) shipped binary/binaries verified." -ForegroundColor Green
} finally {
    if (Test-Path $extract) { Remove-Item $extract -Recurse -Force -ErrorAction SilentlyContinue }
}

Write-Step 'Release artifacts'
Write-Host "Signed installer: $($msi.FullName)"
Write-Host ("Installer size:   {0:N0} bytes" -f $msi.Length)
Write-Host "The installer and every binary it ships are signed and verified." -ForegroundColor Green
Write-Host "Note: src-tauri\target\release\sagedock.exe is deliberately NOT the release" -ForegroundColor DarkGray
Write-Host "binary and is left unsigned by Tauri. Ship the MSI." -ForegroundColor DarkGray
