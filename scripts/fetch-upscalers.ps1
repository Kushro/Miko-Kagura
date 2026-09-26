#Requires -Version 7.0
[CmdletBinding()]
param(
    [string] $Destination = "$PSScriptRoot/../target/release-packaging/binaries",
    [string] $CacheDirectory = "$PSScriptRoot/../target/download-cache"
)
. "$PSScriptRoot/release-common.ps1"
$Destination = [IO.Path]::GetFullPath($Destination)
if (Test-Path $Destination) { throw "Destination must not exist: $Destination" }
New-Item -ItemType Directory $Destination | Out-Null
$lockPath = "$ProjectRoot/packaging/upscalers.lock.json"
$lock = Get-Content $lockPath -Raw | ConvertFrom-Json
foreach ($package in $lock.packages) {
    Write-Host "Fetching $($package.repository) $($package.version)"
    $archive = Get-VerifiedDownload $package.url $package.sha256 $CacheDirectory
    $extraction = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
    try {
        Expand-Archive -LiteralPath $archive -DestinationPath $extraction
        $root = Join-Path $extraction $package.archiveRoot
        if (!(Test-Path "$root/$($package.executable)")) { throw 'Unexpected upstream archive layout.' }
        $target = New-Item -ItemType Directory "$Destination/$($package.id)"
        Get-ChildItem $root -Force | Copy-Item -Destination $target -Recurse
    } finally {
        if (Test-Path $extraction) { Remove-Item $extraction -Recurse -Force }
    }
}
New-Item -ItemType Directory "$Destination/licenses" | Out-Null
foreach ($notice in $lock.notices) {
    $source = Get-VerifiedDownload $notice.url $notice.sha256 $CacheDirectory
    Copy-Item $source "$Destination/licenses/$($notice.file)"
}
Copy-Item $lockPath "$Destination/upscalers.lock.json"
