#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string] $Tag, [Parameter(Mandatory)][long] $ReleaseId)
. "$PSScriptRoot/release-common.ps1"
$null = Get-ReleaseVersion $Tag
if (!$env:GH_REPO) { throw 'GH_REPO is required.' }
$release = gh api "repos/$env:GH_REPO/releases/$ReleaseId" | ConvertFrom-Json -AsHashtable
Assert-NativeSuccess 'Read triggering release'
if ($release.tag_name -cne $Tag -or $release.draft) { throw 'The triggering release was changed or unpublished.' }
if ($release.immutable) { throw 'Cannot attach files to an immutable published release. Upload to a draft before publishing instead.' }
$dist = "$ProjectRoot/dist/$Tag"
$files = @(Get-ChildItem $dist -File | Sort-Object Name)
if ($files.Count -ne 5) { throw 'Expected four packages and SHA256SUMS.txt.' }
$prefix = "Miko-Kagura-$Tag-windows-x64"
$expectedNames = @("$prefix-portable-lite.zip", "$prefix-portable-full.zip", "$prefix-setup-lite.exe", "$prefix-setup-full.exe")
$sums = @(Get-Content "$dist/SHA256SUMS.txt")
if ($sums.Count -ne 4) { throw 'Expected four checksums.' }
foreach ($name in $expectedNames) {
    $line = @($sums | Where-Object { $_ -cmatch ('^[a-f0-9]{64}  ' + [regex]::Escape($name) + '$') })
    if ($line.Count -ne 1 -or !(Test-Path "$dist/$name")) { throw "Missing package or checksum: $name" }
    if ((Get-FileHash "$dist/$name").Hash -ine $line[0].Substring(0, 64)) { throw "Package checksum mismatch: $name" }
}
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
