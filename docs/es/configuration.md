# Configurar modelos y preferencias

[English](../en/configuration.md) | **Español** | [Inicio](../../README.es.md)

Usa el panel para cambiar el procesamiento sin reiniciar el servidor.

## Controles del panel

El panel izquierdo permite seleccionar modelo, escala, reducción de ruido, GPU,
compresión, normalización PNG, dimensión máxima y **Binary Directory**.

El panel derecho contiene las pestañas **Requests**, **Plugins**, **Settings**,
**Errors** y **Storage**. Selecciona una solicitud para comparar las imágenes
original y escalada en tu visor. Storage administra capturas y limpieza automática.
Los controles superiores cambian el tema o limpian la vista actual; los temas
incluyen claro, oscuro, Catppuccin, Nord y Dracula. La barra inferior muestra la
dirección de escucha y el contador de solicitudes. Arrastra el lado izquierdo de
la barra de título personalizada para mover la ventana.

## Modelos compatibles

| Clave del modelo | Familia | Escalas registradas |
| --- | --- | --- |
| `waifu2x` | CUNet | 1, 2 |
| `waifu2x-upconv7` | UpConv7 anime | 2 |
| `waifu2x-photo` | UpConv7 fotografía | 2 |
| `realcugan-se` | Real-CUGAN SE (predeterminado) | 2, 3, 4 |
| `realcugan-pro` | Real-CUGAN Pro | 2, 3 |
| `realcugan-nose` | Real-CUGAN Nose | 2 |
| `realesrgan-anime` | Real-ESRGAN anime video | 2, 3, 4 |
| `realesrgan-photo` | Real-ESRGAN fotografía | 4 |

Las combinaciones de reducción de ruido dependen del motor y de los modelos
disponibles. Comienza con escala `2` y ruido `0` en Real-CUGAN SE. Conserva la
normalización PNG para mantener la compatibilidad con los decodificadores de Android.

## Límite de tamaño de salida

El límite de dimensión reduce proporcionalmente las imágenes grandes con Lanczos3.
El máximo es `16383` píxeles por dimensión, el límite de WebP. **Pixels** configura
el valor directamente; **Device** lo calcula con el tamaño de pantalla, la densidad
de píxeles y el margen de zoom. Un margen de `100%` ajusta a la pantalla; `200%`
conserva detalle para un zoom de 2×.

## Preferencias guardadas y CLI

Los cambios se guardan en `%APPDATA%\Miko-Kagura\settings.json`.
Si `APPDATA` no está disponible, se usa `miko-kagura-settings.json` junto al
ejecutable. Las opciones CLI reemplazan las preferencias para esa ejecución.

```powershell
.\miko-kagura.exe --host 0.0.0.0 --port 8282 --model realcugan-se --scale 2 --noise 0
.\miko-kagura.exe --gpu-id 0 --binary-dir "C:\Tools\upscalers"
```

| Opción | Función / valor inicial |
| --- | --- |
| `--host`, `--port` | Dirección de escucha; `0.0.0.0`, `8282` |
| `--model`, `--scale` / `-s`, `--noise` / `-n` | Modelo y parámetros de procesamiento |
| `--binary-dir` | Carpeta de motores; en su defecto, detectar `binaries/` |
| `--gpu-id` | Índice de GPU; `-1` selecciona automáticamente |
| `--max-size` | Límite del cuerpo de una solicitud en MiB; `20` |
| `--compress` / `-C`, `--compress-level` | Activar oxipng; nivel `0`–`6`, predeterminado `2` |
| `--no-normalize` | Desactivar normalización PNG |
| `--no-webp-compat` | Desactivar límite de dimensión |
| `--webp-max-dimension` | Dimensión máxima de salida; `16383` |
| `--help` / `-h` | Mostrar ayuda del comando |

## Plugins de fuentes

Los plugins reparan imágenes descargadas por el servidor desde fuentes que
reordenan píxeles o cifran bytes. Están desactivados por defecto y solo se aplican
a `/upscale/url` y `/upscale/batch/url`. Las cargas directas ya llegan
decodificadas por el lector y no pasan por los plugins.

| Plugin | Procesamiento |
| --- | --- |
| Comix | Reordenamiento 5×5 con semilla y cifrado LCG/xorshift |
| GigaViewer | Transposición de bloques 4×4 |
| ComiciViewer | Orden de bloques desde el fragmento de URL |
| Clip Studio Reader | Cuadrícula variable desde el fragmento de URL |
| K Manga | Reordenamiento 4×4 derivado de xorshift32 |
| MANGA Plus | Secuencia XOR desde el fragmento de URL |
| Zebrack | XOR usando `#key=<hex>` |
| YnJn / Sokuyomi | Reflejo diagonal 4×4 |
| Cabeceras anti-hotlink | Referer/User-Agent de la fuente |

Conserva los fragmentos de URL al enviar solicitudes. El servidor los elimina
antes de descargar por HTTP. Comix lee las cabeceras de respuesta `x-scramble-*`
y admite imágenes en CDN externas. Los errores aparecen en **Errors**; el
procesamiento continúa con la imagen descargada. Consulta el origen de estas
adaptaciones en [Componentes de terceros](third-party-components.md).
