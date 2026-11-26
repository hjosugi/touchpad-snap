# Signing the Windows binaries

Unsigned executables trigger SmartScreen, and some corporate policies block
them. The release workflow already has an optional signing step; it is off
until a certificate is configured. This is the complete setup.

## What the workflow does

In `.github/workflows/release.yml`, after `cargo build` and before zipping, on
Windows runners only:

1. decode `secrets.WINDOWS_CERT_PFX` (base64) to a temporary `.pfx`,
2. `signtool sign /f <pfx> /p <password> /fd SHA256 /tr <timestamp> /td SHA256`
   `touchpad-snap.exe` and `touchpad-snapw.exe`,
3. fail the build unless `Get-AuthenticodeSignature` reports `Valid`,
4. remove the `.pfx`.

It runs only when the repository variable `WINDOWS_SIGNING` is `true`:

```console
$ gh variable set WINDOWS_SIGNING --body true
$ gh secret set WINDOWS_CERT_PFX < cert.pfx.base64.txt
$ gh secret set WINDOWS_CERT_PASSWORD
```

The `.pfx` must be base64 without line breaks (Linux/macOS:
`base64 -w0 cert.pfx > cert.pfx.base64.txt`; PowerShell:
`[Convert]::ToBase64String([IO.File]::ReadAllBytes('cert.pfx'))`).

## Certificate options

| Option | For | Notes |
|---|---|---|
| **Azure Trusted Signing** | easiest for OSS | Microsoft-managed short-lived certs; no PFX to store. It uses the `azure/trusted-signing-action` and OIDC/federated credentials instead of the PFX step; replace the `Sign (Windows)` step with that action and set the Azure secrets. SmartScreen reputation is immediate. |
| **SignPath** (free for OSS) | no certificate purchase | Submit the two exes between build and zip and let `signpath/github-action-submit-signing-request` wait for the signed files. Replace the step; the rest of the workflow stays the same. |
| **OV/EV certificate as PFX** | what the current step expects | OV builds SmartScreen reputation over downloads; EV (or Azure) skips it. Buy from a CA that allows code signing and export a PFX including the private key. Store it only in the GitHub secret, never in the repo. |

For Azure Trusted Signing the secret names are not fixed by this repo; use the
action's documentation and adapt the step. The PFX path above is the simplest
one to switch on today.

## Verifying a release

On Windows:

```powershell
$zip = 'touchpad-snap-v0.7.0-x86_64-pc-windows-msvc.zip'
Expand-Archive $zip -DestinationPath signed
Get-AuthenticodeSignature .\signed\touchpad-snap-v0.7.0-x86_64-pc-windows-msvc\touchpad-snap.exe |
  Format-List Status, StatusMessage, SignerCertificate
Get-AuthenticodeSignature .\signed\touchpad-snap-v0.7.0-x86_64-pc-windows-msvc\touchpad-snapw.exe |
  Format-List Status
```

Both must say `Valid`, and the signer must be the expected publisher. On
Linux/macOS `osslsigncode verify -in touchpad-snap.exe` works too.

## Testing without tagging

Run the release workflow manually against an existing tag
(`workflow_dispatch` with `tag`), or temporarily set the variable and dispatch
a build. Delete the resulting draft/asset if it was only a test; a real
release should carry the signed executables from its first publication so the
hashes in the package manifests match the signed files.

## Common failures

- `signtool` not found: it comes with the Windows SDK on `windows-latest`;
  for a self-hosted runner install the Windows SDK.
- `The specified PFX password is not correct`: the secret has a trailing
  newline; set it from a file and check with `gh secret list` (values are not
  shown, but you can re-set it).
- `Error: SignerSign() failed` with a timestamp error: retry, or use another
  RFC 3161 timestamp server (`http://timestamp.digicert.com` is the default).
- The signature is invalid after zipping: the zip step must copy the signed
  files, not rebuild them. The current order does that; keep it if you adapt
  the step.
- SmartScreen still warns with an OV certificate: reputation is per
  certificate and builds over downloads; EV/Azure avoid the wait.
