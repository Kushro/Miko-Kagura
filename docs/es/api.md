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
| POST | `/upscale` | Bytes de la imagen | Bytes PNG |
| POST | `/upscale/url` | JSON: `url`, mapa `headers` opcional | Bytes PNG |
| POST | `/upscale/batch` | JSON: arreglo `images` de imágenes base64 | Resultados JSON por imagen |
| POST | `/upscale/batch/url` | JSON: `urls`, mapa compartido `headers` opcional | Resultados JSON por imagen |

`/status` incluye `model`, `model_display_name`, `scale`, `noise`, `gpu`,
`uptime_seconds`, `requests_processed`, `bytes_processed`, `binary_dir` y
`binaries_ready`. Los campos del modelo pueden ser null antes de cargarlo.

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
tienen prioridad sobre las de los plugins. Conserva los fragmentos de URL para
los [plugins de reparación](configuration.md#plugins-de-fuentes).

## Leer resultados por lotes

La respuesta contiene `batch_id` y un arreglo `results`. Cada resultado incluye
`index` y `success`. Un resultado correcto incluye `image` (PNG en base64),
`width`, `height` y `time_ms`; uno fallido incluye `error`. Revisa cada entrada
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
descargan en paralelo; el servidor coordina el acceso al motor.
