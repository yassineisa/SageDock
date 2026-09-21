<#
.SYNOPSIS
    Runs every ignored integration test against an isolated SageDockQA runtime.
.DESCRIPTION
    Imports the pinned image, validates notebooks and Windows integration, then removes
    only the QA registration after validating its name and install path. Restores the
    caller's environment variables even when testing or cleanup fails.
#>
param(
    [string]$Image = (Join-Path $PSScriptRoot '../src-tauri/runtime/sagedock-runtime-sage10.9-x64.tar.xz')
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$resolvedImage = (Resolve-Path -LiteralPath $Image).Path
$name = 'SageDockQA-' + [Guid]::NewGuid().ToString('N')
$scratch = Join-Path $repo ('test-results/fresh-install-' + $name)
New-Item -ItemType Directory -Path $scratch -Force | Out-Null
$previousEnvironment = @{}
foreach ($key in @('SAGEDOCK_QA_DISTRO', 'SAGEDOCK_TEST_IMAGE', 'SAGEDOCK_TEST_APPDATA', 'SAGEDOCK_TEST_WORKSPACE')) {
    $previousEnvironment[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
}
$env:SAGEDOCK_QA_DISTRO = $name
$env:SAGEDOCK_TEST_IMAGE = $resolvedImage
$env:SAGEDOCK_TEST_APPDATA = Join-Path $scratch 'appdata'
$env:SAGEDOCK_TEST_WORKSPACE = Join-Path $scratch 'workspace'
# Only this randomly named QA environment is ever removed. The personal SageDock
# distribution, its workspace, and other distributions are not test targets.
try {
    & cargo test --offline --manifest-path (Join-Path $repo 'src-tauri/Cargo.toml') installs_a_package_and_opens_a_sage_notebook -- --ignored --nocapture --test-threads=1
    if ($LASTEXITCODE -ne 0) {
        throw 'Fresh-install verification failed. Test files are retained in test-results.'
    }
    # Reuse the verified QA runtime for the remaining OS and notebook integration checks.
    & cargo test --offline --lib --manifest-path (Join-Path $repo 'src-tauri/Cargo.toml') -- --ignored --skip installs_a_package_and_opens_a_sage_notebook --nocapture --test-threads=1
    if ($LASTEXITCODE -ne 0) {
        throw 'Additional integration checks failed. Test files are retained in test-results.'
    }
} finally {
    try {
        if ($name -notmatch '^SageDockQA-[a-f0-9]{32}$') { throw 'Unsafe test name; refusing cleanup.' }
        $registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss'
        if (Test-Path $registry) {
            foreach ($key in Get-ChildItem $registry) {
                $entry = Get-ItemProperty $key.PSPath
                if ($entry.DistributionName -eq $name) {
                    $expected = [IO.Path]::GetFullPath((Join-Path $env:SAGEDOCK_TEST_APPDATA "runtime/$name"))
                    $actual = [IO.Path]::GetFullPath(($entry.BasePath -replace '^\\\\\?\\',''))
                    if ($actual -ne $expected -or -not $expected.StartsWith($scratch,[StringComparison]::OrdinalIgnoreCase)) { throw 'QA path mismatch; refusing cleanup.' }
                    & wsl.exe --unregister $name
                    if ($LASTEXITCODE -ne 0) { throw "QA cleanup failed; retained $name" }
                }
            }
        }
    } finally {
        foreach ($key in $previousEnvironment.Keys) {
            [Environment]::SetEnvironmentVariable($key, $previousEnvironment[$key], 'Process')
        }
    }
}
