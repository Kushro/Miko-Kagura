#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string] $Tag)
. "$PSScriptRoot/release-common.ps1"
$null = Get-ReleaseVersion $Tag
$reg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\com.kushro.miko-kagura'
$desktopLink = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Miko-Kagura.lnk'
$menu = Join-Path ([Environment]::GetFolderPath('Programs')) 'Miko-Kagura'
if ((Test-Path $reg) -or (Test-Path $desktopLink) -or (Test-Path $menu)) {
    throw 'An existing Miko-Kagura install/shortcut was found. Run installer tests in a disposable Windows environment.'
}
$work = Join-Path ([IO.Path]::GetTempPath()) ('miko-installer-test-' + [Guid]::NewGuid())
$install = Join-Path $work 'installed'
New-Item -ItemType Directory $work | Out-Null
$dist = "$ProjectRoot/dist/$Tag"
$prefix = "Miko-Kagura-$Tag-windows-x64"
function Invoke-Setup([string] $Path, [string] $Arguments) {
    $process = Start-Process $Path -ArgumentList $Arguments -PassThru
    if (!$process.WaitForExit(180000)) { $process.Kill(); throw "Installer timed out: $Path" }
    if ($process.ExitCode -ne 0) { throw "Installer failed: $Path ($($process.ExitCode))" }
}
function Assert-Installed([string] $Edition) {
    $reference = "$work/$Edition/Miko-Kagura"
    foreach ($file in Get-ChildItem $reference -File -Recurse) {
        $relative = [IO.Path]::GetRelativePath($reference, $file.FullName)
        $actual = Join-Path $install $relative
        if (!(Test-Path $actual) -or (Get-FileHash $actual).Hash -ne (Get-FileHash $file.FullName).Hash) {
            throw "Installed file differs from portable payload: $relative"
        }
    }
}
function Uninstall-TestApp {
    Invoke-Setup "$install/uninstall.exe" "/S _?=$install"
    if (Test-Path "$install/miko-kagura.exe") { throw 'Application was not removed.' }
    if (Test-Path $reg) { throw 'Uninstall registry entry remains.' }
    if ((Test-Path $desktopLink) -or (Test-Path $menu)) { throw 'Shortcuts remain after uninstall.' }
}
try {
    foreach ($edition in @('lite', 'full')) {
        Expand-Archive "$dist/$prefix-portable-$edition.zip" "$work/$edition"
        Assert-Payload "$work/$edition/Miko-Kagura" $edition
    }
    # A clean Lite install, then upgrade to Full, reinstall Full, then update via Lite.
    foreach ($edition in @('lite', 'full', 'full', 'lite')) {
        Invoke-Setup "$dist/$prefix-setup-$edition.exe" "/S /D=$install"
        Assert-Installed $edition
        if (!(Test-Path "$install/user-file.txt")) {
            Set-Content "$install/user-file.txt" 'Preserve this user-owned file.'
        }
    }
    Assert-Installed 'full'
    New-Item -ItemType Directory -Force "$install/binaries/custom" | Out-Null
    Set-Content "$install/binaries/custom/user-model.txt" 'External model placeholder.'
    Uninstall-TestApp
    foreach ($file in @('user-file.txt', 'binaries/custom/user-model.txt')) {
        if (!(Test-Path "$install/$file")) { throw "Uninstaller removed user content: $file" }
    }
    if (Test-Path "$install/binaries/realcugan/realcugan-ncnn-vulkan.exe") { throw 'Bundled engine was not removed.' }
    # Clean Full installation after removing only this test's directory.
    Remove-Item $install -Recurse -Force
    Invoke-Setup "$dist/$prefix-setup-full.exe" "/S /D=$install"
    Assert-Installed 'full'
    Uninstall-TestApp
    Write-Host 'Installer checks passed: Lite, Full, reinstall, edition transitions and user-file preservation.'
} finally {
    if (Test-Path "$install/uninstall.exe") {
        Invoke-Setup "$install/uninstall.exe" "/S _?=$install"
    }
    Remove-Item $work -Recurse -Force
}
