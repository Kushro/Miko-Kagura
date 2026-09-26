# Componentes de terceros

[English](../en/third-party-components.md) | **Español** | [Inicio](../../README.es.md)

Los paquetes Full incluyen ejecutables NCNN Vulkan y pesos de modelos de sus
proyectos originales. Lite permite usar tus propias copias. Se descargan durante
el empaquetado y permanecen fuera del historial de Git.

## Versiones de motores

| Componente | Paquete fijado | Origen |
| --- | --- | --- |
| Waifu2x | Windows 20250915 | [nihui/waifu2x-ncnn-vulkan](https://github.com/nihui/waifu2x-ncnn-vulkan/releases/tag/20250915) |
| Real-CUGAN | Windows 20220728 | [nihui/realcugan-ncnn-vulkan](https://github.com/nihui/realcugan-ncnn-vulkan/releases/tag/20220728) |
| Real-ESRGAN | Windows 20220424, release v0.2.5.0 | [xinntao/Real-ESRGAN](https://github.com/xinntao/Real-ESRGAN/releases/tag/v0.2.5.0) |

El [manifiesto de distribución](../../packaging/upscalers.lock.json) registra URLs
exactas, hashes SHA-256, modelos esperados y fuentes de licencias adicionales.
Full conserva el contenido original de los paquetes y añade licencias en
`binaries/licenses/`. Los motores incluyen una copia del manifiesto.

## Licencias y atribución

Las implementaciones NCNN de Waifu2x y Real-CUGAN usan MIT. Sus proyectos de
modelos originales son nagadomi/waifu2x y Real-CUGAN de bilibili/ailab.
La implementación NCNN de Real-ESRGAN incluye avisos MIT de Xintao Wang y nihui;
el proyecto Real-ESRGAN original usa BSD-3-Clause. NCNN y sus dependencias tienen
avisos propios. Consulta el registro central de [avisos de terceros](../../THIRD_PARTY_NOTICES.md).

Los plugins de reparación son adaptaciones Rust de interceptores Keiyoushi bajo
Apache-2.0. La licencia no comercial de Miko-Kagura se aplica a su código original;
no reemplaza los permisos de los componentes de terceros. Los textos legales
originales se conservan en su idioma de origen.

## Actualizar las versiones fijadas

1. Selecciona una release concreta del origen, no una URL variable `latest`.
2. Descarga el paquete Windows y calcula SHA-256. Compáralo con el hash del origen si existe.
3. Revisa ejecutable, estructura de modelos, licencias y avisos de dependencias.
4. Actualiza el manifiesto y las dos versiones de esta guía.
5. Compila Full, verifica todos los modelos registrados y procesa una imagen con cada motor en una GPU.

Los archivos antiguos del origen no publican hashes. Los checksums del manifiesto
se calcularon desde las descargas oficiales; no son firmas del proyecto original.
No se incluyen controladores de GPU ni Vulkan SDK. WebView2 se instala por
separado mediante el instalador inicial de Microsoft cuando hace falta.
