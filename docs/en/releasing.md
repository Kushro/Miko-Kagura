# Build and publish a release

**English** | [Español](../es/releasing.md) | [Home](../../README.md)

The release workflow builds Windows x64 Portable Lite/Full and Setup Lite/Full,
plus Linux x64 Portable Lite/Full tarballs, a Full AppImage and a Full .deb, then
attaches them and one combined `SHA256SUMS.txt` to the release that triggered it.
Every job runs on the runners chosen by `runner.yml` (Avrea by default).

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
early for them. If immutability is required, fill a draft first (see
[Attach to a draft before publishing](#attach-to-a-draft-before-publishing)). See
[GitHub release events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#release)
and [immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).

### Attach to a draft before publishing

To review the downloads before anyone sees the release, create it as a draft and
run the workflow with `attach` checked:

```powershell
gh release create v0.1.0 --repo Kushro/Miko-Kagura --verify-tag --draft --title "Miko-Kagura v0.1.0" --generate-notes
gh workflow run release.yml --repo Kushro/Miko-Kagura -f tag=v0.1.0 -f attach=true
```

The run builds and tests the tagged commit, not the workflow ref, and uploads to
the only draft for that tag; it refuses a published release. When you later
publish the draft, `release: published` finds every package already attached and
skips the build and upload, so the published files are the ones you reviewed.

## Linux packages and the AUR

The `linux` job runs on Avrea's Ubuntu 24.04 runner, so the packages require
glibc 2.39 or newer. To reproduce it on Ubuntu 24.04 with the dependencies listed
in the workflow, Dioxus CLI 0.7.9 and bun:

```bash
bash scripts/build-release-linux.sh v0.1.0
bash scripts/test-linux-packages.sh v0.1.0
```

Packages are in `dist/v0.1.0-linux`. The build downloads the engines pinned in
`packaging/upscalers-linux-x64.lock.json`, plus pinned `appimagetool` and AppImage
runtime builds. The .deb gets its `Depends` from `dpkg-shlibdeps`. The test script
needs Docker (`DOCKER=podman` also works). It installs the .deb and runs the
AppImage on Ubuntu 24.04, builds and installs the AUR package on Arch, checks
library resolution, and waits for `/health` to report bundled engines.

The AppImage and the .deb carry PNG icons, AppStream metadata
(`packaging/linux/miko-kagura.metainfo.xml`, filled with the release version and
date) and, in the AppImage, `X-AppImage-Version`, so AppImage managers show the
icon and version. The PNGs in `packaging/linux/icons` are committed; after editing
`packaging/linux/miko-kagura.svg`, regenerate them with
`bash scripts/render-linux-icons.sh` (needs `rsvg-convert`).

The workflow keeps the tested `PKGBUILD` and `.SRCINFO` as the
`aur-miko-kagura-bin-<tag>` artifact. They are not release assets. After the
release is published, copy both files into your clone of
`ssh://aur@aur.archlinux.org/miko-kagura-bin.git`, then commit and push. To
regenerate them on Arch from a checkout of the tag, run:

```bash
scripts/update-aur.sh v0.1.0 SHA256SUMS.txt aur-out
cd aur-out && makepkg --printsrcinfo > .SRCINFO
```

Publish only stable versions to the AUR. If you do publish a prerelease,
`0.2.0-beta.1` becomes `pkgver=0.2.0beta.1`, which pacman orders before `0.2.0`.

## Dry runs and upload retries

Use **Actions → Release packages → Run workflow** with a version tag
value to build without publishing. Unless `attach` is checked, this builds the selected workflow ref,
not a tag selected by that text field. The version must match that ref's manifests.
Each platform's packages and `SHA256SUMS.txt` are retained as an Actions
artifact for 14 days.

Uploads skip byte-identical existing assets and fail on conflicting names.
They never replace downloads silently. If an upload partially fails, download
the original Actions artifacts into `dist/<tag>` (Windows) and `dist/<tag>-linux`
(Linux) and retry with that exact payload:

```powershell
$env:GH_REPO = 'Kushro/Miko-Kagura'
$releaseId = gh release view v0.1.0 --json databaseId --jq .databaseId
pwsh -File scripts/publish-release.ps1 -Tag v0.1.0 -ReleaseId $releaseId
```

Add `-Draft` when the release is still a draft.

Rebuilding can change archive/installer timestamps, so a full rerun may correctly
report a conflict. Inspect existing assets before explicitly removing any.
