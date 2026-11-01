# Referencia de la API HTTP

[English](../en/api.md) | **Español** | [Inicio](../../README.es.md)

La URL base predeterminada es `http://<ip-de-la-computadora>:8282`. El servidor
procesa imágenes para Miko; no sirve manga ni contenido de fuentes. Úsalo en
una red de confianza: la API HTTP no tiene autenticación.

## Endpoints

| Método | Ruta | Solicitud | Respuesta |
| --- | --- | --- | --- |
| GET | `/health` | Ninguna | `status`, `upscaler_ready` |
| GET | `/status` | Ninguna | Modelo, escala, ruido, GPU, contadores y ruta de motores |
| POST | `/model` | JSON: `model`, opcionales `scale` (2), `noise` (0) | Modelo seleccionado y `loaded` |
| POST | `/upscale` | Bytes de la imagen | Bytes PNG o WebP |
| POST | `/upscale/url` | JSON: `url`, mapa `headers` opcional | Bytes PNG o WebP |
| POST | `/upscale/batch` | JSON: arreglo `images` de imágenes base64 | Resultados JSON por imagen |
| POST | `/upscale/batch/url` | JSON: `urls`, mapa compartido `headers` opcional | Resultados JSON por imagen |

`/status` incluye `model`, `model_display_name`, `scale`, `noise`, `gpu`,
`uptime_seconds`, `requests_processed`, `bytes_processed`, `binary_dir`,
`binaries_ready` e `input_formats`. Los campos del modelo pueden ser null antes
de cargarlo.

`input_formats` enumera los formatos de imagen que aceptan como entrada todos los
endpoints de escalado: `png`, `jpeg`, `webp`, `gif`, `bmp`, `tiff` y `avif`. Los
clientes pueden enviar las páginas en esos formatos tal cual y convertir antes
cualquier otro (HEIF, JPEG XL). Los servidores más antiguos no incluyen el campo
y no aceptan AVIF.

Las imágenes se devuelven en PNG por defecto, o en WebP con pérdida cuando el
[formato de salida](configuration.md#formato-de-salida) es WebP. Revisa
`Content-Type` (`image/png` o `image/webp`) o decodifica por contenido.

## Enviar una imagen

Ejecuta desde una carpeta con `input.png`, reemplazando el host por tu servidor:

```powershell
curl.exe --fail-with-body -X POST http://192.168.1.100:8282/upscale --data-binary "@input.png" -o output.png
curl.exe --fail-with-body http://192.168.1.100:8282/status
```

Las respuestas correctas de una imagen incluyen `X-Upscale-Model`,
`X-Upscale-Scale`, `X-Upscale-Noise` y `X-Process-Time-Ms`. El límite inicial del
cuerpo de una solicitud es 20 MiB; se configura con `--max-size`.

## Enviar una URL

```powershell
$body = @{ url = 'https://example.com/image.png'; headers = @{ Referer = 'https://example.com/' } } | ConvertTo-Json
Invoke-WebRequest -Method Post -Uri 'http://192.168.1.100:8282/upscale/url' -ContentType 'application/json' -Body $body -OutFile 'output.png'
```

Reemplaza la URL de ejemplo por una imagen accesible. Las cabeceras del cliente
tienen prioridad sobre las de los plugins. El servidor descarga como máximo
cuatro imágenes a la vez de cada host y reintenta hasta dos veces más los
timeouts, errores de conexión y HTTP 408/429/5xx, esperando lo que pida
`Retry-After` (hasta 10 segundos) o demoras crecientes con un componente
aleatorio. Los desafíos de Cloudflare y otros bloqueos fallan de inmediato,
porque repetir no los resuelve. Conserva los fragmentos de URL para los [plugins
de reparación](configuration.md#plugins-de-fuentes).

## Leer resultados por lotes

La respuesta contiene `batch_id` y un arreglo `results`. Cada resultado incluye
`index` y `success`. Un resultado correcto incluye `image` en base64, su
`format` (`png` o `webp`), `width`, `height` y `time_ms`; uno fallido incluye
`error`. Revisa cada entrada
en lugar de asumir que todo el lote fue correcto.

```json
{
  "batch_id": 1,
  "results": [
    { "index": 0, "success": false, "error": "Example processing error" }
  ]
}
```

La ruta de lotes de imágenes admite hasta 32 veces el límite configurado para
una solicitud individual, para alojar varias imágenes base64. Los lotes por URL
descargan en paralelo y encolan cada imagen para escalar en cuanto llega. El
servidor ejecuta el motor de a un trabajo por vez y procesa en una sola
ejecución las imágenes en espera con los mismos ajustes, así que las solicitudes
concurrentes cuestan menos que las secuenciales. `X-Process-Time-Ms` y `time_ms`
indican el tiempo de motor atribuido a cada imagen, sin contar la espera en cola.

### Recibir lotes en streaming

Envía `Accept: application/x-ndjson` a cualquiera de las rutas de lotes para
recibir cada resultado en cuanto está listo, sin esperar al lote completo. La
respuesta es JSON delimitado por saltos de línea: una primera línea con
`batch_id` y `count`, y luego un objeto de resultado por línea, en orden de
finalización y no de solicitud. Asocia cada resultado con su solicitud por
`index`. Cerrar la conexión cancela las imágenes que aún no terminaron. Sin esa
cabecera, los lotes responden con el documento JSON único de arriba.

```text
{"batch_id":1,"count":3}
{"index":1,"success":true,"image":"…","width":1800,"height":2600,"time_ms":410}
{"index":0,"success":true,"image":"…","width":1800,"height":2600,"time_ms":410}
{"index":2,"success":false,"error":"Example processing error"}
```
