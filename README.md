# Miko-Kagura

**English** | [Español](README.es.md)

Miko-Kagura is the desktop companion for [Miko](https://github.com/Kushro/miko).
It runs Miko's remote image upscaling on your computer's GPU. Use the dashboard
to choose models, adjust image processing, and inspect requests from the reader.

## Choose a download

Open [Releases](https://github.com/Kushro/Miko-Kagura/releases) and choose a
**Windows x64** package. Each packaged release provides these four variants:

| Filename suffix | Includes engines and models | Best for |
| --- | --- | --- |
| `setup-full.exe` | Yes | First installation (recommended) |
| `portable-full.zip` | Yes | Running without installation |
| `setup-lite.exe` | No | Installing or updating with existing engines |
| `portable-lite.zip` | No | Using your own engines without installation |

Full includes Waifu2x, Real-CUGAN and Real-ESRGAN engines and models downloaded
from their official sources. Lite includes the same application and features.
GitHub's **Source code** archives contain source code, not a runnable package.

## Requirements

- Windows x64 and a GPU with an up-to-date Vulkan-capable driver.
- Microsoft Edge WebView2 Runtime. The installer installs it if missing;
  that step requires Internet access. Portable users install it separately.
- Miko and the computer reachable on the same trusted network.

## Get started

1. Install **Setup Full**, or extract **Portable Full** and open `miko-kagura.exe`.
2. Keep the default `realcugan-se` model for your first request.
3. Find the computer's local IP address, for example `192.168.1.100`.
4. Set Miko's remote-upscaling server address to `http://192.168.1.100:8282`.
5. Check `http://192.168.1.100:8282/health`. `upscaler_ready` should be `true`.
6. Enable remote upscaling in Miko and open an image.

The API has no authentication. Use a trusted network and allow the selected
port through the firewall for that network. Read [Installation](docs/en/installation.md)
for Lite setup, updates, checksums, and uninstall instructions.

## Documentation

| Goal | Guide |
| --- | --- |
| Install, update, or choose engines | [Installation](docs/en/installation.md) |
| Choose models and change preferences | [Configuration](docs/en/configuration.md) |
| Resolve connection or processing problems | [Troubleshooting](docs/en/troubleshooting.md) |
| Integrate another client | [HTTP API](docs/en/api.md) |
| Build and understand the application | [Development](docs/en/development.md) |
| Produce and publish packages | [Releasing](docs/en/releasing.md) |
| Inspect bundled dependencies | [Third-party components](docs/en/third-party-components.md) |

Portable means **no installation required**. Preferences are stored in
`%APPDATA%\Miko-Kagura\settings.json`, including when running a portable package.

## License

Original project code is available under [PolyForm Noncommercial 1.0.0](LICENSE):
`Required Notice: Copyright (c) 2026 Octavio`. This is source-available software
for permitted noncommercial use. Third-party components retain their own
licenses and permissions. See [Third-party notices](THIRD_PARTY_NOTICES.md).
