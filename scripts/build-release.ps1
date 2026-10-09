#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string] $Tag, [switch] $SkipTests)
. "$PSScriptRoot/release-common.ps1"
$previousDefines = $env:MIKO_INSTALLER_DEFINES
Push-Location $ProjectRoot
try {
    $release = Get-ReleaseVersion $Tag
    if (!(dx --version).StartsWith('dioxus 0.7.9 ')) { throw 'Install Dioxus CLI 0.7.9.' }
    $work = "$ProjectRoot/target/release-packaging"
    $dist = "$ProjectRoot/dist/$Tag"
    # Only generated directories owned by this script are reset.
    foreach ($path in @($work, $dist)) {
        if (Test-Path $path) { Remove-Item $path -Recurse -Force }
        New-Item -ItemType Directory $path | Out-Null
    }
    bun install --frozen-lockfile
    Assert-NativeSuccess 'bun install'
    bun run tailwind:build
    Assert-NativeSuccess 'Tailwind build'
    if (!$SkipTests) {
        cargo test --locked
        Assert-NativeSuccess 'cargo test'
    }
    dx build --platform desktop --release --locked
    Assert-NativeSuccess 'dx build'
    & "$PSScriptRoot/fetch-upscalers.ps1" -Destination "$work/binaries"

    $app = "$ProjectRoot/target/dx/miko-kagura/release/windows/app"
    foreach ($edition in @('lite', 'full')) {
        $payload = "$work/$edition/Miko-Kagura"
        New-Item -ItemType Directory -Force $payload | Out-Null
        Get-ChildItem $app -Force | Copy-Item -Destination $payload -Recurse
        foreach ($item in @('LICENSE', 'LICENSES', 'README.md', 'README.es.md', 'THIRD_PARTY_NOTICES.md', 'docs')) {
            Copy-Item "$ProjectRoot/$item" $payload -Recurse
        }
        New-Item -ItemType Directory "$payload/packaging" | Out-Null
        Copy-Item "$ProjectRoot/packaging/upscalers.lock.json" "$payload/packaging/"
        if ($edition -eq 'full') { Copy-Item "$work/binaries" "$payload/binaries" -Recurse }
        Assert-Payload $payload $edition
        $prefix = "Miko-Kagura-$Tag-windows-x64"
        Compress-Archive -Path $payload -DestinationPath "$dist/$prefix-portable-$edition.zip" -CompressionLevel Optimal

        # Separate ownership lists let Lite updates preserve Full's engines.
        foreach ($group in @('core', 'engines')) {
            $files = @(Get-ChildItem $payload -Recurse -File | ForEach-Object { [IO.Path]::GetRelativePath($payload, $_.FullName) } | Where-Object {
                if ($group -eq 'engines') { $_.StartsWith('binaries\') } else { !$_.StartsWith('binaries\') }
            } | Sort-Object)
            $dirs = @($files | ForEach-Object {
                $parent = [IO.Path]::GetDirectoryName($_)
                while ($parent) { $parent; $parent = [IO.Path]::GetDirectoryName($parent) }
            } | Sort-Object -Unique | Sort-Object Length -Descending)
            $lines = @($files | ForEach-Object { "F|$_" }) + @($dirs | ForEach-Object { "D|$_" })
            [IO.File]::WriteAllText("$work/$group-files.txt", ($lines -join "`r`n") + "`r`n", [Text.UTF8Encoding]::new($false))
        }
        function ConvertTo-NsisLiteral([string] $Text) { $Text.Replace('$', '$$').Replace('"', '$\"') }
        $defines = @(
            '!define PAYLOAD "' + (ConvertTo-NsisLiteral ($payload.Replace('/', '\'))) + '"'
            '!define CORE_FILES "' + (ConvertTo-NsisLiteral "$work/core-files.txt".Replace('/', '\')) + '"'
            '!define ENGINE_FILES "' + (ConvertTo-NsisLiteral "$work/engines-files.txt".Replace('/', '\')) + '"'
            '!define WINDOWS_VERSION "' + $release.WindowsVersion + '"'
        )
        if ($edition -eq 'full') { $defines += '!define FULL_EDITION' }
        Set-Content "$work/installer.nsh" ($defines -join "`n") -Encoding utf8BOM
        $env:MIKO_INSTALLER_DEFINES = "$work/installer.nsh".Replace('/', '\')
        $bundleOut = "$work/installer-$edition"
        dx bundle --platform desktop --release --locked --package-types nsis --out-dir $bundleOut
        Assert-NativeSuccess "dx bundle ($edition)"
        $installers = @(Get-ChildItem $bundleOut -Filter '*.exe' -File)
        if ($installers.Count -ne 1) { throw 'Expected exactly one NSIS installer.' }
        Copy-Item $installers[0].FullName "$dist/$prefix-setup-$edition.exe"
    }
    $artifacts = @(Get-ChildItem $dist -File | Sort-Object Name)
    if ($artifacts.Count -ne 4) { throw 'Expected four release packages.' }
    $artifacts | ForEach-Object { "{0}  {1}" -f (Get-FileHash $_.FullName).Hash.ToLowerInvariant(), $_.Name } |
        Set-Content "$dist/SHA256SUMS.txt" -Encoding ascii
    Write-Host "Release packages: $dist"
} finally {
    $env:MIKO_INSTALLER_DEFINES = $previousDefines
    Pop-Location
}
