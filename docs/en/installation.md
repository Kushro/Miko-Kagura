# Install Miko-Kagura

**English** | [Español](../es/installation.md) | [Home](../../README.md)

Install the Windows x64 application and connect Miko to its upscaling server.
For a first installation, choose **Setup Full** on the
[releases page](https://github.com/Kushro/Miko-Kagura/releases).

## Before you start

Install a current GPU driver with Vulkan support. The application also needs
[WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/).
Setup installs WebView2 when missing using Microsoft's online bootstrapper.
Full includes upscaling engines and model weights, not a GPU driver or an
offline WebView2 runtime.

## Use an installer

1. Download `Miko-Kagura-v<VERSION>-windows-x64-setup-full.exe`.
2. Run it and choose English or Spanish for the installer.
3. Select an installation folder. The default is `%LOCALAPPDATA%\Programs\Miko-Kagura`.
4. Open Miko-Kagura from the Start menu or desktop shortcut.

Installation is per user. The application interface keeps its existing language;
the language selector applies to the installer and documentation.

## Use a portable package

1. Download `Miko-Kagura-v<VERSION>-windows-x64-portable-full.zip`.
2. Extract the entire archive to a writable folder.
3. Open `Miko-Kagura\miko-kagura.exe`.

Keep `assets/` and, for Full, `binaries/` beside the executable. Preferences
are shared with the installed edition through `%APPDATA%\Miko-Kagura`.

## Configure Lite engines

Lite includes the application but no upscaling executables or weights.
Choose an existing engine directory in the dashboard's **Binary Directory**
control, or start the application with an absolute path:

```powershell
.\miko-kagura.exe --binary-dir "C:\Tools\upscalers"
```

For manual installation, extract the official packages listed in
[Third-party components](third-party-components.md) into this layout:

```text
binaries/
├── waifu2x/      # waifu2x-ncnn-vulkan.exe and its models-* directories
├── realcugan/    # realcugan-ncnn-vulkan.exe and models-se/pro/nose
└── realesrgan/   # realesrgan-ncnn-vulkan.exe and models/
```

Keep each package's licenses. Select the parent `binaries/` directory in the
dashboard. Its executables and model directories must remain together.

## Connect Miko

1. Find the computer's local IPv4 address with `ipconfig` in PowerShell.
2. Allow TCP port `8282` through the firewall on your trusted network.
3. Open `http://<computer-ip>:8282/health` from the device running Miko.
4. Set that host and port as Miko's remote-upscaling server address.

The server binds to `0.0.0.0` by default. It has no authentication; do not expose
it directly to the Internet. Readiness confirms engine discovery, not a GPU
inference test. Send an image to verify the complete setup.

## Update or switch editions

Close Miko-Kagura before updating. Run the new installer using the same install
folder. Lite and Full use one application identity and one set of preferences.

- **Lite → Full:** installs the bundled engines and models.
- **Full → Lite:** updates the application and preserves previously installed engines.
- **Full → Full:** replaces the engine files owned by the previous Full installer.
- **Portable:** extract to a new folder. With Lite, point **Binary Directory** to your existing engines.

Installer-managed files can be replaced during updates. Store custom models
outside the installation folder to manage their lifetime independently.
Saved absolute engine paths continue to take precedence; update them if you
move the engine folder. Prereleases display their full version but use the
numeric `major.minor.patch.0` Windows file version. Install the intended version
explicitly; the installer does not enforce SemVer upgrade ordering.

## Verify a download

Download `SHA256SUMS.txt` from the same release. Compare the checksum with:

```powershell
Get-FileHash .\Miko-Kagura-v0.1.0-windows-x64-setup-full.exe -Algorithm SHA256
```

Use your downloaded filename. The hash must match its entry in `SHA256SUMS.txt`.
Packages are currently unsigned.

## Uninstall

Use Windows **Installed apps** or run `uninstall.exe` in the application folder.
The uninstaller removes recorded application and engine files. User-added files,
external engine directories, and `%APPDATA%\Miko-Kagura` preferences remain.
For a portable package, close the application and delete its extracted folder.
