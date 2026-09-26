# Shared by the local packaging commands and GitHub Actions. Requires PowerShell 7.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:ProjectRoot = Split-Path $PSScriptRoot -Parent

function Assert-NativeSuccess([string] $Operation) {
    if ($LASTEXITCODE -ne 0) { throw "$Operation failed with exit code $LASTEXITCODE" }
}

function Get-ReleaseVersion([string] $Tag) {
    $number = '(0|[1-9][0-9]*)'
    $identifier = '(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
    $pattern = "^v$number\.$number\.$number(?:-($identifier(?:\.$identifier)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
    if ($Tag -cnotmatch $pattern) { throw "Expected a SemVer tag starting with lowercase v, got '$Tag'." }
    $parts = ($Tag.Substring(1) -split '[-+]')[0].Split('.')
    foreach ($part in $parts) {
        if ([long]$part -gt 65535) { throw 'Windows version components must be <= 65535.' }
    }
    $metadata = cargo metadata --no-deps --locked --format-version 1 --manifest-path "$script:ProjectRoot/Cargo.toml" | ConvertFrom-Json
    Assert-NativeSuccess 'cargo metadata'
    $package = @($metadata.packages | Where-Object name -EQ 'miko-kagura')[0]
    $version = $Tag.Substring(1)
    if ($package.version -cne $version) { throw "Tag $Tag does not match Cargo.toml version $($package.version)." }
    $frontend = Get-Content "$script:ProjectRoot/package.json" -Raw | ConvertFrom-Json
    if ($frontend.version -cne $version) { throw 'Update package.json to the same version as Cargo.toml.' }
    [pscustomobject]@{ Tag = $Tag; Version = $version; WindowsVersion = ($parts -join '.') + '.0' }
}

function Get-VerifiedDownload([string] $Url, [string] $Sha256, [string] $CacheDirectory) {
    if ($Sha256 -notmatch '^[a-f0-9]{64}$') { throw "Invalid SHA-256 for $Url" }
    if (![Uri]::IsWellFormedUriString($Url, [UriKind]::Absolute) -or !($Url.StartsWith('https://'))) {
        throw "Expected an HTTPS source: $Url"
    }
    New-Item -ItemType Directory -Force $CacheDirectory | Out-Null
    $file = Join-Path $CacheDirectory ($Sha256 + '-' + [IO.Path]::GetFileName(([Uri]$Url).AbsolutePath))
    if (!(Test-Path $file)) {
        $partial = "$file.partial"
        Invoke-WebRequest -Uri $Url -OutFile $partial -MaximumRetryCount 3 -RetryIntervalSec 5
        if ((Get-FileHash $partial -Algorithm SHA256).Hash -ine $Sha256) {
            Remove-Item $partial
            throw "SHA-256 mismatch: $Url"
        }
        Move-Item $partial $file -Force
    }
    if ((Get-FileHash $file -Algorithm SHA256).Hash -ine $Sha256) { throw "Corrupt cached download: $file" }
    return $file
}

function Assert-Payload([string] $Directory, [ValidateSet('lite', 'full')][string] $Edition) {
    foreach ($relative in @('miko-kagura.exe', 'LICENSE', 'README.md', 'README.es.md', 'THIRD_PARTY_NOTICES.md', 'assets', 'docs/en/installation.md', 'docs/es/installation.md')) {
        if (!(Test-Path (Join-Path $Directory $relative))) { throw "Missing payload file: $relative" }
    }
    if (@(Get-ChildItem "$Directory/assets" -File -Recurse).Count -eq 0) { throw 'The assets directory is empty.' }
    if ($Edition -eq 'lite') {
        if (Test-Path "$Directory/binaries") { throw 'Lite unexpectedly contains upscaling engines.' }
        return
    }
    $lock = Get-Content "$script:ProjectRoot/packaging/upscalers.lock.json" -Raw | ConvertFrom-Json
    foreach ($package in $lock.packages) {
        $base = "$Directory/binaries/$($package.id)"
        foreach ($file in @($package.executable) + @($package.requiredFiles)) {
            if (!(Test-Path "$base/$file" -PathType Leaf)) { throw "Missing Full resource: $base/$file" }
        }
        foreach ($model in $package.modelDirectories) {
            $params = @(Get-ChildItem "$base/$model" -Filter '*.param' -File)
            if ($params.Count -eq 0) { throw "Missing model parameters: $base/$model" }
            foreach ($param in $params) {
                if (!(Test-Path ([IO.Path]::ChangeExtension($param.FullName, '.bin')))) { throw "Missing model weights for $param" }
            }
            foreach ($weights in Get-ChildItem "$base/$model" -Filter '*.bin' -File) {
                if (!(Test-Path ([IO.Path]::ChangeExtension($weights.FullName, '.param')))) { throw "Missing model parameters for $weights" }
            }
        }
    }
    foreach ($notice in $lock.notices) {
        $path = "$Directory/binaries/licenses/$($notice.file)"
        if (!(Test-Path $path) -or (Get-FileHash $path).Hash -ine $notice.sha256) { throw "Missing or altered notice: $path" }
    }
}
