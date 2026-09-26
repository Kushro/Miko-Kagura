# Compilar y contribuir

[English](../en/development.md) | **Español** | [Inicio](../../README.es.md)

Esta guía describe las herramientas de desarrollo y publicación para Windows x64.

## Preparar herramientas

Instala Rust mediante rustup, las herramientas C++ de MSVC, Bun **1.4.2**,
PowerShell **7** y WebView2 Runtime. `rust-toolchain.toml` fija Rust **1.94.1**.
Instala Dioxus CLI **0.7.9** desde su
[release oficial](https://github.com/DioxusLabs/dioxus/releases/tag/v0.7.9),
o compílalo con:

```powershell
cargo install dioxus-cli --version 0.7.9 --locked
```

## Ejecutar y compilar

Desde la raíz del repositorio:

```powershell
bun install --frozen-lockfile
bun run tailwind:build
dx serve --platform desktop
```

Pasa las opciones de ejecución después de `--`, por ejemplo:

```powershell
dx serve --platform desktop -- --binary-dir "C:\Tools\upscalers" --gpu-id 0
dx build --platform desktop --release --locked
```

La aplicación release se genera en `target/dx/miko-kagura/release/windows/app/`.
Conserva juntos el ejecutable y los assets. Dioxus recopila los recursos
`asset!()`; `cargo run` por sí solo no genera la estructura equivalente.

## Ejecutar comprobaciones

```powershell
cargo test --locked -j 2
bun run docs:check
```

Para generar las distribuciones completas, consulta [Publicación](releasing.md).
La inferencia necesita una GPU compatible con Vulkan; las pruebas unitarias
no verifican por sí solas la combinación de motor y controlador instalada.

## Arquitectura

El servidor axum se ejecuta en un hilo dedicado con un runtime Tokio. La ventana
Dioxus ocupa el hilo principal. Ambos comparten `Arc<AppState>` y un canal
Tokio `broadcast` con mensajes `ServerEvent` para actualizar el panel.
Los ejecutables NCNN realizan la inferencia en subprocesos.

| Código | Responsabilidad |
| --- | --- |
| `src/config.rs` | Valores iniciales y registro de modelos |
| `src/upscaler.rs` | Detección de motores, subprocesos y GPU |
| `src/server.rs` | Rutas HTTP, procesamiento y estado compartido |
| `src/settings.rs` | Preferencias JSON |
| `src/history.rs`, `src/cache.rs` | Comparaciones, capturas y temporales |
| `src/clamp.rs` | Cálculo de la dimensión de salida |
| `src/plugins.rs` | Reparación de imágenes de fuentes |
| `src/ui/`, `src/main.rs` | Panel, CLI e inicio |
| `scripts/`, `packaging/` | Automatización de releases y plantilla del instalador |

## Mantener ambos idiomas de la documentación

Actualiza juntos los archivos en inglés y español. Conserva comandos, claves
de modelos, nombres de archivos y etiquetas de la interfaz al traducir.
Cada página debe enlazar a su traducción y al README del idioma correspondiente.

Usa un objetivo por guía, requisitos antes de los pasos numerados, frases breves
en voz activa y enlaces descriptivos. Los README deben centrarse en descargas y
primeros pasos. `bun run docs:check` verifica paridad de páginas, estructura de
títulos, enlaces de idioma y destinos locales; revisa manualmente la traducción.

Estas convenciones siguen las
[buenas prácticas de GitHub Docs](https://docs.github.com/es/contributing/writing-for-github-docs/best-practices-for-github-docs)
y las [recomendaciones para traducir contenido](https://docs.github.com/es/contributing/writing-for-github-docs/writing-content-to-be-translated).
