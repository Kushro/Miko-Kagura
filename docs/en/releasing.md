# Build and publish a release

**English** | [Español](../es/releasing.md) | [Home](../../README.md)

The release workflow builds Windows x64 Portable Lite/Full and Setup Lite/Full,
then attaches them and `SHA256SUMS.txt` to the release that triggered it.

## Prepare a version

1. Update `Cargo.toml` and `package.json` to the same SemVer version.
2. Refresh `Cargo.lock` with `cargo check` and keep dependency changes intentional.
3. Update both documentation languages and the engine lock manifest if needed.
4. Commit and push the changes, including the workflow, before tagging.

For example, version `0.1.0` uses tag `v0.1.0`; version `0.2.0-beta.1` uses
`v0.2.0-beta.1`. Tags must start with lowercase `v` and match the package version.
Windows numeric version components must be at most `65535`. Prerelease/build
suffixes remain in filenames and display strings; the NSIS numeric version is
`major.minor.patch.0`.

## Build locally

Use the [development prerequisites](development.md) and run PowerShell 7:

```powershell
pwsh -File scripts/build-release.ps1 -Tag v0.1.0
```

The script runs tests, builds Tailwind and Dioxus, downloads hash-pinned engines,
then stages separate Lite/Full payloads and builds NSIS installers. Downloads are
cached in `target/download-cache`. Generated work is in `target/release-packaging`;
packages are in `dist/v0.1.0`. Only those generated directories are reset.
`-SkipTests` is available for local packaging iterations after tests have passed.

The custom NSIS template uses generated payload definitions and the
`MIKO_INSTALLER_DEFINES` environment variable. Use the packaging script instead
of invoking `dx bundle` directly. Dioxus fetches its
NSIS tools and Microsoft's WebView2 bootstrapper during bundling.

## Validate packages

```powershell
bun run docs:check
pwsh -File scripts/test-installers.ps1 -Tag v0.1.0
```

The installer smoke test uses a temporary install directory and verifies Lite,
Full, reinstall, both edition transitions, and preservation of user-added files.
It refuses to run when this user's Miko-Kagura installation or shortcuts already
exist. Prefer a disposable Windows runner. It does not delete user preferences.

Before declaring a release ready, launch an extracted Full package outside the
checkout, confirm the styles render, check `/health`, and upscale a real image
on a Vulkan-capable GPU. Hosted runners do not establish GPU compatibility.

## Publish on GitHub

Create the tag and publish its release from the GitHub UI or your authenticated CLI:

```powershell
git tag v0.1.0
git push origin v0.1.0
gh release create v0.1.0 --repo Kushro/Miko-Kagura --verify-tag --title "Miko-Kagura v0.1.0" --generate-notes
```

`release: published` covers stable releases and prereleases. A draft or tag push
alone does not start packaging. The workflow skips releases without a `v` prefix,
checks out the triggering commit, and validates the version before building.
Tests and packaging run as parallel jobs; a final publish job uploads the files
only after both succeed. The release initially has no compiled downloads.

GitHub's automatic `GITHUB_TOKEN` gives only the publish job `contents: write`; no personal token
is needed for uploading. Creating a release from a different workflow using
`GITHUB_TOKEN` does not normally trigger this workflow; use an appropriate GitHub
App token or invoke the packaging steps in that workflow instead.

Published **immutable releases cannot receive new assets**. This workflow fails
early for them. If immutability is required, change the release process to build,
upload to a draft, then publish. See
[GitHub release events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#release)
and [immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).

## Dry runs and upload retries

Use **Actions → Release Windows packages → Run workflow** with a version tag
value to build without publishing. This builds the selected workflow ref,
not a tag selected by that text field. The version must match that ref's manifests.
The five output files are retained as an Actions artifact for 14 days.

Uploads skip byte-identical existing assets and fail on conflicting names.
They never replace downloads silently. If an upload partially fails, download
the original Actions artifact into `dist/<tag>` and retry with that exact payload:

```powershell
$env:GH_REPO = 'Kushro/Miko-Kagura'
$releaseId = gh release view v0.1.0 --json databaseId --jq .databaseId
pwsh -File scripts/publish-release.ps1 -Tag v0.1.0 -ReleaseId $releaseId
```

Rebuilding can change archive/installer timestamps, so a full rerun may correctly
report a conflict. Inspect existing assets before explicitly removing any.
