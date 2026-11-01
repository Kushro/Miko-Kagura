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
parallel and queue each one for upscaling as soon as it arrives. The server
runs the engine one job at a time and processes images waiting with the same
settings in a single engine run, so concurrent requests are cheaper than
sequential ones. `X-Process-Time-Ms` and `time_ms` report the engine time
attributed to each image, excluding time spent waiting in the queue.

### Stream batch results

Send `Accept: application/x-ndjson` to either batch route to receive each result
as soon as it is ready instead of waiting for the whole batch. The response is
newline-delimited JSON: a first line with `batch_id` and `count`, then one
result object per line, in completion order rather than request order. Match
results to requests by `index`. Closing the connection cancels the images that
haven't finished. Without that header, batches answer with the single JSON
document above.

```text
{"batch_id":1,"count":3}
{"index":1,"success":true,"image":"…","width":1800,"height":2600,"time_ms":410}
{"index":0,"success":true,"image":"…","width":1800,"height":2600,"time_ms":410}
{"index":2,"success":false,"error":"Example processing error"}
```
