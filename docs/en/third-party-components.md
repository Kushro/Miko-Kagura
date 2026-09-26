# Third-party components

**English** | [Español](../es/third-party-components.md) | [Home](../../README.md)

Full packages include upstream NCNN Vulkan executables and model weights.
Lite packages let you supply them yourself. They are downloaded during packaging
and remain outside Git history.

## Engine versions

| Component | Pinned package | Upstream |
| --- | --- | --- |
| Waifu2x | Windows 20250915 | [nihui/waifu2x-ncnn-vulkan](https://github.com/nihui/waifu2x-ncnn-vulkan/releases/tag/20250915) |
| Real-CUGAN | Windows 20220728 | [nihui/realcugan-ncnn-vulkan](https://github.com/nihui/realcugan-ncnn-vulkan/releases/tag/20220728) |
| Real-ESRGAN | Windows 20220424, release v0.2.5.0 | [xinntao/Real-ESRGAN](https://github.com/xinntao/Real-ESRGAN/releases/tag/v0.2.5.0) |

The [distribution manifest](../../packaging/upscalers.lock.json) records exact
URLs, SHA-256 hashes, expected models and supplemental license sources.
Full preserves the upstream archive contents and adds license texts under
`binaries/licenses/`. A copy of the manifest accompanies the engines.

## Licenses and attribution

Waifu2x and Real-CUGAN NCNN wrappers use MIT licenses. Their original model
projects are nagadomi/waifu2x and bilibili/ailab's Real-CUGAN. The Real-ESRGAN
NCNN wrapper includes MIT notices for Xintao Wang and nihui; the original
Real-ESRGAN project uses BSD-3-Clause. NCNN and its dependencies have their own
notices. See the central [third-party notices](../../THIRD_PARTY_NOTICES.md).

Source-repair plugins are Rust adaptations of Keiyoushi interceptors under
Apache-2.0. Miko-Kagura's noncommercial license applies to its original code;
it does not replace the permissions granted for third-party components.
Original legal texts are retained in their source language.

## Refresh the engine lock

1. Select a specific upstream release, not a moving `latest` URL.
2. Download the Windows package and calculate SHA-256. Compare an upstream digest when available.
3. Inspect the executable, model layout, licenses, and dependency notices.
4. Update the manifest and both versions of this guide.
5. Build Full, verify all registered models, and run an image through each engine on a GPU.

Older upstream assets do not publish hashes. Their manifest checksums were
calculated from the official downloads; they are not upstream signatures.
GPU drivers and the Vulkan SDK are not bundled. WebView2 is installed separately
by Microsoft's bootstrapper when necessary.
