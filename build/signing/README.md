# Release signing

SageDock's Windows binaries and installer are signed with **Microsoft Azure Artifact
Signing** (Trusted Signing) by [`scripts/release.ps1`](../../scripts/release.ps1).

## `signing-metadata.json` contains no secrets, and must never contain any

It holds four keys. Three are identifiers rather than credentials; the fourth is a list of
authentication methods to skip, explained below.

| Field                    | Value                                | What it is                                             |
| ------------------------ | ------------------------------------ | ------------------------------------------------------ |
| `Endpoint`               | `https://eus.codesigning.azure.net/` | The public regional service endpoint.                  |
| `CodeSigningAccountName` | `YassinEisa`                         | The Artifact Signing account name.                     |
| `CertificateProfileName` | `YassinEisaPublicSigning`            | The certificate profile to sign with.                  |
| `ExcludeCredentials`     | a list of provider names             | Which auth methods **not** to try, not any credential. |

Knowing all of it grants nothing. Signing additionally requires an Azure identity holding
the **Code Signing Certificate Profile Signer** role on that profile.

The file is committed deliberately: it is build configuration a reviewer should be able to
read, and keeping it in the tree means a release cannot be signed under a different
identity without that change showing up in a diff.

The schema is fixed by `Azure.CodeSigning.Dlib.dll`, which rejects unknown keys, so the
explanation lives in this file rather than as comments inside the JSON.

## Where the signing key is

In Microsoft's service. It cannot be exported, and this repository never sees it.

There is no `.pfx`, no `.p12`, no thumbprint-addressed local certificate and no private
key anywhere in the build. A certificate thumbprint is **not** a signing key and is not
used as one here.

## How authentication works

`Azure.CodeSigning.Dlib.dll` authenticates with `DefaultAzureCredential`, which at signing
time picks up whatever Azure identity is already available on the machine, for a local
release, the developer's existing `az login` session.

Consequently the build:

- never prompts for a password;
- never calls `az account get-access-token` or otherwise materialises a token;
- never writes a token, refresh token, client secret or credential cache into the tree.

`scripts/release.ps1` verifies the active subscription with `az account show` before it
signs anything, and that command prints only a name, a subscription id and a user name.

The metadata's optional `AccessToken` field is deliberately **not** used. Supplying a token
there would mean minting and writing a credential into a tracked file; the whole point of
the CLI-session path is that no token is ever materialised.

### Why `ExcludeCredentials` is set

`DefaultAzureCredential` tries providers in a fixed order, and
**`ManagedIdentityCredential` comes before `AzureCliCredential`**. On a developer machine
that is not an Azure VM there is no instance metadata endpoint to answer it, so it retries
against `169.254.169.254` until it gives up. Observed directly here: signing one 8 MB
binary hung at `Submitting digest for signing...` for over 25 minutes before it was
killed. With the exclusions below in place the same file signed in **4.2 seconds**.

So the list is a real fix, not tidying. It also serves a second purpose:
`InteractiveBrowserCredential` is excluded so a release build can never silently turn into
a password prompt, it either uses the existing CLI session or it fails.

If a CI pipeline ever signs these artifacts, it will need a different list, typically
excluding nothing, so `EnvironmentCredential` or a workload identity can be picked up.

## Local-only build inputs

The signing client itself (`Microsoft.Trusted.Signing.Client`, ~15 MB of Microsoft
tooling) is downloaded on demand into `.signing/`, which is git-ignored. It is a build
tool, not source, and is re-fetched by the release script when absent.

## Reproducing a release

```powershell
az login                              # if not already signed in
pwsh -File scripts/release.ps1        # or: powershell -ExecutionPolicy Bypass -File ...
```

The script fails the release if signing or signature verification fails at any step.
