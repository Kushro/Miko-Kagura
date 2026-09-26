# Configure models and preferences

**English** | [Español](../es/configuration.md) | [Home](../../README.md)

Use the dashboard to change image processing without restarting the server.

## Dashboard controls

The left panel selects the model, scale, noise level, GPU, compression, PNG
normalization, maximum output dimension, and **Binary Directory**.

The right panel contains **Requests**, **Plugins**, **Settings**, **Errors**,
and **Storage** tabs. Select a request to compare the original and upscaled
images in your image viewer. Storage controls manage captures and automatic
cleanup. The top-right controls change the theme or clear the current view;
themes include light, dark, Catppuccin, Nord, and Dracula. The bottom bar shows
the listening address and request count. Drag the left side of the custom title
bar to move the window.

## Supported models

| Model key | Family | Registered scales |
| --- | --- | --- |
| `waifu2x` | CUNet | 1, 2 |
| `waifu2x-upconv7` | UpConv7 anime | 2 |
| `waifu2x-photo` | UpConv7 photo | 2 |
| `realcugan-se` | Real-CUGAN SE (default) | 2, 3, 4 |
| `realcugan-pro` | Real-CUGAN Pro | 2, 3 |
| `realcugan-nose` | Real-CUGAN Nose | 2 |
| `realesrgan-anime` | Real-ESRGAN anime video | 2, 3, 4 |
| `realesrgan-photo` | Real-ESRGAN photo | 4 |

Available denoising combinations depend on the engine and supplied model files.
Start with scale `2` and noise `0` on Real-CUGAN SE. Keep PNG normalization on
for compatibility with Android image decoders.

## Output size limit

The maximum-dimension clamp proportionally resizes oversized output with Lanczos3.
Its upper bound is WebP's `16383` pixels per dimension. **Pixels** sets the limit
directly; **Device** derives it from screen size, pixel density, and zoom headroom.
Headroom `100%` fits the screen; `200%` retains detail for a 2× zoom.

## Saved preferences and CLI

Preferences are saved as you change them to `%APPDATA%\Miko-Kagura\settings.json`.
If `APPDATA` is unavailable, the fallback is `miko-kagura-settings.json` beside
the executable. CLI options override saved settings for that launch.

```powershell
.\miko-kagura.exe --host 0.0.0.0 --port 8282 --model realcugan-se --scale 2 --noise 0
.\miko-kagura.exe --gpu-id 0 --binary-dir "C:\Tools\upscalers"
```

| Option | Purpose / initial default |
| --- | --- |
| `--host`, `--port` | Listening address; `0.0.0.0`, `8282` |
| `--model`, `--scale` / `-s`, `--noise` / `-n` | Model and processing parameters |
| `--binary-dir` | Engine root; otherwise auto-detect `binaries/` |
| `--gpu-id` | GPU index; `-1` means automatic selection |
| `--max-size` | Single request body limit in MiB; `20` |
| `--compress` / `-C`, `--compress-level` | Enable oxipng; preset `0`–`6`, default `2` |
| `--no-normalize` | Disable PNG normalization |
| `--no-webp-compat` | Disable output dimension clamping |
| `--webp-max-dimension` | Maximum output dimension; `16383` |
| `--help` / `-h` | Show command help |

## Source plugins

Plugins repair images downloaded by the server from sources that scramble
pixels or encrypt image bytes. They are disabled by default and apply only to
`/upscale/url` and `/upscale/batch/url`. Direct uploads are already decoded by
the reader and bypass plugins.

| Plugin | Processing |
| --- | --- |
| Comix | Seeded 5×5 tile shuffle and LCG/xorshift byte cipher |
| GigaViewer | 4×4 block transpose |
| ComiciViewer | Tile order from the URL fragment |
| Clip Studio Reader | Variable grid from the URL fragment |
| K Manga | xorshift32-derived 4×4 shuffle |
| MANGA Plus | XOR keystream from the URL fragment |
| Zebrack | XOR using `#key=<hex>` |
| YnJn / Sokuyomi | 4×4 diagonal mirror |
| Anti-hotlink headers | Source-specific Referer/User-Agent |

Preserve URL fragments when sending requests. The server removes them before
the HTTP download. Comix reads `x-scramble-*` response headers and supports
images on external CDNs. Plugin errors appear in **Errors**; processing then
continues with the downloaded image. See [Third-party components](third-party-components.md)
for the origin of these adaptations.
