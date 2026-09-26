# Troubleshoot Miko-Kagura

**English** | [Español](../es/troubleshooting.md) | [Home](../../README.md)

Start with the **Errors** tab and the `/health` endpoint to distinguish a
connection problem from a missing engine or failed inference.

## Miko cannot connect

1. Keep Miko-Kagura open and check its listening port.
2. On the computer, open `http://127.0.0.1:8282/health`.
3. On the phone, use the computer's LAN IP instead of `localhost`.
4. Check the firewall and Wi-Fi client isolation settings.

If another process uses the port, start with `--port 8283` and change the address
in Miko to match. See [Installation](installation.md).

## The server reports not ready

An `upscaler_ready: false` response means the current model or executable is
unavailable. With Lite, configure **Binary Directory**. With Full, check that
`binaries/` was extracted completely. A previously saved absolute engine path
can point at an old installation; select the current directory.

## Vulkan or model loading fails

Update the GPU driver and verify that its Vulkan support works. Try
`--gpu-id 0` if automatic selection chooses an unsuitable device. Confirm that
the engine's model directory contains both `.param` and `.bin` files.
Use the model, scale, and noise combination supported by that engine.

The GPU name shown in the dashboard can come from Windows' integrated adapter
even when inference uses another GPU. NCNN's inference output is the relevant
device information; the dashboard name alone does not determine device selection.

## The window is unstyled or fails to open

Extract the whole package, including `assets/`. When building from source, use
`dx build` or `dx serve` so Dioxus collects the stylesheets. Plain `cargo run`
does not prepare those assets. Install WebView2 Runtime if the window cannot open.

## Images look scrambled or comparisons lack labels

For URL requests, enable the matching source plugin and preserve the URL
fragment. Check **Errors** for missing or failed repairs. For direct image
uploads, decoding belongs to the reader. See [Configuration](configuration.md).

Comparison labels require an available TrueType font, such as
`C:\Windows\Fonts\arial.ttf`. Without a font, the side-by-side image has no labels.

## Installation or updates fail

Close the application before installing or uninstalling. If WebView2 setup fails,
install the runtime from Microsoft, then run Setup again. The installer may need
Internet access for that runtime even with the Full edition.

If a problem persists, include the package name, Windows version, GPU/driver,
model, and relevant error text in an
[issue](https://github.com/Kushro/Miko-Kagura/issues). Remove private URLs or headers.
