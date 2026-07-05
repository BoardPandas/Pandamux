# Winget distribution (issue #32)

These manifests publish wmux to the [Windows Package Manager](https://learn.microsoft.com/windows/package-manager/),
so users can install/upgrade with:

```powershell
winget install BoardPandas.wmux
winget upgrade BoardPandas.wmux
```

> **Status:** the manifests (`BoardPandas.wmux.*.yaml`) and
> `.github/workflows/winget.yml` are repointed to the `BoardPandas.wmux`
> identifier and the `BoardPandas/Pandamux` repo, but the package has **not**
> been bootstrapped in `microsoft/winget-pkgs` yet, so `winget install` will not
> work until the one-time submission below is done. The version and
> `InstallerSha256` pinned in the manifests are stale placeholders from the
> upstream fork; regenerate them against the real release zip at bootstrap.

wmux ships as a **portable zip** (no code-signing — an unsigned NSIS installer
trips SmartScreen *harder* than a zip extraction), so the manifest models it as
`InstallerType: zip` + `NestedInstallerType: portable`, exposing a `wmux`
command alias on PATH that launches the app.

> ⚠️ Winget improves install **UX**, not **trust**: an unsigned binary still
> trips SmartScreen on first run. Clearing that needs code-signing (Azure
> Trusted Signing ≈ $10/mo, or the Microsoft Store which signs for free): a
> separate, owner-gated decision tracked in issue #32.

## Files

| File | Winget `ManifestType` |
|------|-----------------------|
| `BoardPandas.wmux.yaml` | `version` |
| `BoardPandas.wmux.installer.yaml` | `installer` |
| `BoardPandas.wmux.locale.en-US.yaml` | `defaultLocale` |

## Bootstrap (one-time)

`winget-releaser` only **updates** an existing package, so the first version
must be submitted by hand:

1. Fork [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs).
2. Copy these three files (with the real `InstallerSha256`, see below) into
   `manifests/b/BoardPandas/wmux/<version>/`.
3. Validate locally: `winget validate --manifest manifests/b/BoardPandas/wmux/<version>`
   and `winget install --manifest ...` in a sandbox.
4. Open a PR to `microsoft/winget-pkgs`.

Compute the installer hash from the release zip:

```powershell
(Get-FileHash .\wmux-0.8.6-win-x64.zip -Algorithm SHA256).Hash
```

## Ongoing releases

After the bootstrap PR merges, `.github/workflows/winget.yml` opens a winget-pkgs
PR automatically on every published GitHub release (requires the one-time
`WINGET_TOKEN` secret + a winget-pkgs fork — see the workflow header).
