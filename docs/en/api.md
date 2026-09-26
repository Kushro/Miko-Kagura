# HTTP API reference

**English** | [Español](../es/api.md) | [Home](../../README.md)

The default base URL is `http://<computer-ip>:8282`. The server processes images
for Miko; it does not serve manga or source content. Use it on a trusted network:
the HTTP API has no authentication.

## Endpoints

| Method | Path | Request | Response |
| --- | --- | --- | --- |
| GET | `/health` | None | `status`, `upscaler_ready` |
| GET | `/status` | None | Model, scale, noise, GPU, counters and engine path |
| POST | `/model` | JSON: `model`, optional `scale` (2), `noise` (0) | Selected model and `loaded` |
| POST | `/upscale` | Raw image bytes | PNG bytes |
| POST | `/upscale/url` | JSON: `url`, optional `headers` map | PNG bytes |
| POST | `/upscale/batch` | JSON: `images` array of base64 images | JSON batch results |
| POST | `/upscale/batch/url` | JSON: `urls`, optional shared `headers` map | JSON batch results |

`/status` includes `model`, `model_display_name`, `scale`, `noise`, `gpu`,
`uptime_seconds`, `requests_processed`, `bytes_processed`, `binary_dir`, and
`binaries_ready`. Model-related fields can be null before a model is loaded.

## Send an image

Run from a folder containing `input.png`, replacing the host with your server:

```powershell
curl.exe --fail-with-body -X POST http://192.168.1.100:8282/upscale --data-binary "@input.png" -o output.png
curl.exe --fail-with-body http://192.168.1.100:8282/status
```

Successful single-image responses carry `X-Upscale-Model`, `X-Upscale-Scale`,
`X-Upscale-Noise`, and `X-Process-Time-Ms` headers. The default single request
body limit is 20 MiB, configurable with `--max-size`.

## Send a URL

```powershell
$body = @{ url = 'https://example.com/image.png'; headers = @{ Referer = 'https://example.com/' } } | ConvertTo-Json
Invoke-WebRequest -Method Post -Uri 'http://192.168.1.100:8282/upscale/url' -ContentType 'application/json' -Body $body -OutFile 'output.png'
```

Replace the example URL with an accessible image. Client-provided headers take
precedence over plugin defaults. Preserve source URL fragments for the
[repair plugins](configuration.md#source-plugins).

## Read batch results

A batch response has `batch_id` and a `results` array. Each result includes
`index` and `success`. A successful result includes base64 PNG `image`, `width`,
`height`, and `time_ms`; a failed result includes `error`. Inspect each entry
rather than assuming the entire batch succeeded.

```json
{
  "batch_id": 1,
  "results": [
    { "index": 0, "success": false, "error": "Example processing error" }
  ]
}
```

The image-batch route allows a body up to 32 times the configured single-request
limit to accommodate multiple base64 images. URL batches download images in
parallel; actual engine access is coordinated by the server.
