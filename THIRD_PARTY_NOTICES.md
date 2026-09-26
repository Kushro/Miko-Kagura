# Third-party notices

[English guide](docs/en/third-party-components.md) | [Guía en español](docs/es/third-party-components.md)

Miko-Kagura's `LICENSE` applies to the original project code by Octavio, not
to third-party components. A new Git history does not remove upstream rights,
license conditions, or attribution. Keep applicable notices when redistributing
modified code or installing external runtimes.

## Source-adapted image plugins

The source-specific descrambling algorithms in `src/plugins.rs` were ported
from Kotlin interceptors in [Keiyoushi extensions-source](https://github.com/keiyoushi/extensions-source).
Keiyoushi extensions-source is distributed under the Apache License, Version 2.0;
its license text is in [`LICENSES/Apache-2.0.txt`](LICENSES/Apache-2.0.txt).
These are Rust adaptations, not an endorsement by Keiyoushi. Retain the
upstream attribution and review the licenses and notices for individual
extensions when reusing their code. The Apache-licensed portions retain their
own permissions, including commercial rights; Miko-Kagura's noncommercial
license applies only where Octavio holds the relevant rights.

## Distributed executable packages and model weights

The `binaries/` directory is not tracked in Git. **Full release packages include
engines and models** downloaded from the pinned official sources in
[packaging/upscalers.lock.json](packaging/upscalers.lock.json). Lite omits them.
Packaging preserves upstream notices and adds original license texts downloaded
from hash-pinned source revisions to `binaries/licenses/`.

| Directory | Upstream project | License material |
| --- | --- | --- |
| `binaries/waifu2x/` | [waifu2x-ncnn-vulkan](https://github.com/nihui/waifu2x-ncnn-vulkan) | Packaged MIT notice (nihui); supplemental MIT notice for [nagadomi/waifu2x](https://github.com/nagadomi/waifu2x). |
| `binaries/realcugan/` | [realcugan-ncnn-vulkan](https://github.com/nihui/realcugan-ncnn-vulkan) | Packaged MIT notice (nihui); supplemental MIT notice for [bilibili Real-CUGAN](https://github.com/bilibili/ailab/tree/main/Real-CUGAN). |
| `binaries/realesrgan/` | [Real-ESRGAN](https://github.com/xinntao/Real-ESRGAN) / [NCNN implementation](https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan) | Supplemental MIT notices for Xintao Wang and nihui; BSD-3-Clause notice for Real-ESRGAN and its supplied models. |
| Within the engines | [Tencent/ncnn](https://github.com/Tencent/ncnn) | Supplemental upstream license and dependency notices in `binaries/licenses/ncnn.txt`. |

The supplemental material also preserves notices for libwebp (including PATENTS),
libjpeg-turbo/IJG, libpng, zlib-ng, stb and win32dirent. For the latter two,
the original source headers carry their license text. This software is based
in part on the work of the Independent JPEG Group.

Packages are redistributed with their original model files. No separately
downloaded community weights are included. GPU drivers and the Vulkan SDK are
not distributed. Microsoft WebView2 is a separate runtime installed through
Microsoft's bootstrapper when needed.

Cargo and Bun dependencies retain their own licenses. The original legal texts
are not translated or replaced by the explanatory EN/ES guides. Attribution does
not imply endorsement by any upstream project.
