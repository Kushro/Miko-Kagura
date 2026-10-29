#Requires -Version 7.0
[CmdletBinding()]
# -Draft attaches to a draft before it is published; without it the release must be published.
param([Parameter(Mandatory)][string] $Tag, [Parameter(Mandatory)][long] $ReleaseId, [switch] $Draft)
. "$PSScriptRoot/release-common.ps1"
$null = Get-ReleaseVersion $Tag
if (!$env:GH_REPO) { throw 'GH_REPO is required.' }
$release = gh api "repos/$env:GH_REPO/releases/$ReleaseId" | ConvertFrom-Json -AsHashtable
Assert-NativeSuccess 'Read triggering release'
if ($release.tag_name -cne $Tag) { throw 'The release was retagged.' }
if ($release.draft -and !$Draft) { throw 'The triggering release was unpublished.' }
if (!$release.draft -and $Draft) { throw 'The release is already published; publishing it uploads the packages.' }
if ($release.immutable) { throw 'Cannot attach files to an immutable published release. Upload to a draft before publishing instead.' }
$dist = "$ProjectRoot/dist/$Tag"
$linuxDist = "$ProjectRoot/dist/$Tag-linux"
$prefix = "Miko-Kagura-$Tag-windows-x64"
$linuxPrefix = "Miko-Kagura-$Tag-linux-x64"
$platforms = @(
    @{ Directory = $dist; Names = @("$prefix-portable-lite.zip", "$prefix-portable-full.zip", "$prefix-setup-lite.exe", "$prefix-setup-full.exe") }
    @{ Directory = $linuxDist; Names = @("$linuxPrefix-portable-lite.tar.gz", "$linuxPrefix-portable-full.tar.gz", "$linuxPrefix-full.AppImage", "$linuxPrefix-full.deb") }
)
# Each platform job ships its own SHA256SUMS.txt; verify both, then publish one combined file.
foreach ($platform in $platforms) {
    $directory = $platform.Directory
    if (@(Get-ChildItem $directory -File).Count -ne 5) { throw "Expected four packages and SHA256SUMS.txt in $directory." }
    $sums = @(Get-Content "$directory/SHA256SUMS.txt")
    if ($sums.Count -ne 4) { throw "Expected four checksums in $directory." }
    foreach ($name in $platform.Names) {
        $line = @($sums | Where-Object { $_ -cmatch ('^[a-f0-9]{64}  ' + [regex]::Escape($name) + '$') })
        if ($line.Count -ne 1 -or !(Test-Path "$directory/$name")) { throw "Missing package or checksum: $name" }
        if ((Get-FileHash "$directory/$name").Hash -ine $line[0].Substring(0, 64)) { throw "Package checksum mismatch: $name" }
    }
}
$combined = @(Get-Content "$dist/SHA256SUMS.txt") + @(Get-Content "$linuxDist/SHA256SUMS.txt") | Sort-Object { $_.Substring(66) }
foreach ($name in $platforms[1].Names) { Move-Item "$linuxDist/$name" $dist }
Set-Content "$dist/SHA256SUMS.txt" $combined -Encoding ascii
$files = @(Get-ChildItem $dist -File | Sort-Object Name)
if ($files.Count -ne 9) { throw 'Expected eight packages and SHA256SUMS.txt.' }
$pending = @()
$temp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory $temp | Out-Null
try {
    # Preflight all collisions before uploading anything. Never silently replace an asset.
    foreach ($file in $files) {
        $existing = @($release.assets | Where-Object name -CEQ $file.Name)
        if ($existing.Count -gt 0) {
            gh release download $Tag --repo $env:GH_REPO --pattern $file.Name --dir $temp
            Assert-NativeSuccess 'Download existing asset'
            if ((Get-FileHash $file.FullName).Hash -ne (Get-FileHash "$temp/$($file.Name)").Hash) {
                throw "Existing asset differs: $($file.Name). Inspect it before deleting or replacing it."
            }
            Write-Host "Already uploaded with matching checksum: $($file.Name)"
        } else { $pending += $file.FullName }
    }
    foreach ($file in $pending) {
        gh release upload $Tag $file --repo $env:GH_REPO
        Assert-NativeSuccess 'Upload release asset'
    }
} finally { Remove-Item $temp -Recurse -Force }
