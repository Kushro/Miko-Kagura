# Instalar Miko-Kagura

[English](../en/installation.md) | **Español** | [Inicio](../../README.es.md)

Instala la aplicación para Windows x64 y conecta Miko a su servidor de escalado.
Para la primera instalación, elige **Setup Full** en la
[página de releases](https://github.com/Kushro/Miko-Kagura/releases).

## Antes de comenzar

Instala un controlador de GPU actualizado con soporte Vulkan. La aplicación
también necesita [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/).
Setup instala WebView2 si falta mediante el instalador en línea de Microsoft.
Full incluye motores y pesos de modelos, pero no un controlador de GPU ni
un runtime WebView2 sin conexión.

## Usar un instalador

1. Descarga `Miko-Kagura-v<VERSION>-windows-x64-setup-full.exe`.
2. Ejecútalo y elige inglés o español para el instalador.
3. Selecciona la carpeta de instalación. La predeterminada es `%LOCALAPPDATA%\Programs\Miko-Kagura`.
4. Abre Miko-Kagura desde el menú Inicio o el acceso directo del escritorio.

La instalación es por usuario. La interfaz de la aplicación conserva su idioma
actual; la selección de idioma se aplica al instalador y a la documentación.

## Usar un paquete portable

1. Descarga `Miko-Kagura-v<VERSION>-windows-x64-portable-full.zip`.
2. Extrae todo el archivo en una carpeta con permisos de escritura.
3. Abre `Miko-Kagura\miko-kagura.exe`.

Conserva `assets/` y, para Full, `binaries/` junto al ejecutable. Las preferencias
se comparten con la edición instalada mediante `%APPDATA%\Miko-Kagura`.

## Configurar motores en Lite

Lite incluye la aplicación, pero no ejecutables ni pesos de escalado.
Elige una carpeta de motores existente en el control **Binary Directory**
del panel, o inicia la aplicación con una ruta absoluta:

```powershell
.\miko-kagura.exe --binary-dir "C:\Tools\upscalers"
```

Para instalarlos manualmente, extrae los paquetes oficiales de
[Componentes de terceros](third-party-components.md) con esta estructura:

```text
binaries/
├── waifu2x/      # waifu2x-ncnn-vulkan.exe y sus carpetas models-*
├── realcugan/    # realcugan-ncnn-vulkan.exe y models-se/pro/nose
└── realesrgan/   # realesrgan-ncnn-vulkan.exe y models/
```

Conserva las licencias de cada paquete. Selecciona la carpeta principal
`binaries/` en el panel. Los ejecutables deben permanecer junto a sus modelos.

## Conectar Miko

1. Consulta la dirección IPv4 local de la computadora con `ipconfig` en PowerShell.
2. Permite el puerto TCP `8282` en el firewall para tu red de confianza.
3. Abre `http://<ip-de-la-computadora>:8282/health` desde el dispositivo con Miko.
4. Configura ese host y puerto como dirección del servidor de escalado remoto de Miko.

El servidor escucha en `0.0.0.0` por defecto. No tiene autenticación; evita
exponerlo directamente a Internet. El estado de disponibilidad confirma que se
encontró el motor, no que se haya probado la GPU. Envía una imagen para verificar
el funcionamiento completo.

## Actualizar o cambiar de edición

Cierra Miko-Kagura antes de actualizar. Ejecuta el nuevo instalador y usa la misma
carpeta. Lite y Full comparten la identidad de la aplicación y las preferencias.

- **Lite → Full:** instala los motores y modelos incluidos.
- **Full → Lite:** actualiza la aplicación y conserva los motores instalados.
- **Full → Full:** reemplaza los archivos de motores administrados por el instalador Full anterior.
- **Portable:** extrae en una carpeta nueva. Con Lite, selecciona los motores existentes en **Binary Directory**.

Las actualizaciones pueden reemplazar los archivos administrados por el
instalador. Guarda tus modelos personalizados fuera de la carpeta de instalación
para administrarlos por separado. Las rutas absolutas guardadas siguen teniendo
prioridad; actualízalas si mueves los motores. Las prereleases muestran su versión
completa, pero usan `major.minor.patch.0` como versión numérica de Windows.
Instala explícitamente la versión deseada; el instalador no impone el orden SemVer.

## Verificar una descarga

Descarga `SHA256SUMS.txt` de la misma release. Compara el checksum con:

```powershell
Get-FileHash .\Miko-Kagura-v0.1.0-windows-x64-setup-full.exe -Algorithm SHA256
```

Usa el nombre del archivo descargado. El hash debe coincidir con su entrada en
`SHA256SUMS.txt`. Los paquetes actualmente no tienen firma digital.

## Desinstalar

Usa **Aplicaciones instaladas** de Windows o ejecuta `uninstall.exe` en la carpeta
de la aplicación. Se eliminan los archivos registrados de la aplicación y sus
motores. Se conservan los archivos añadidos por el usuario, los motores externos
y las preferencias en `%APPDATA%\Miko-Kagura`. Para un paquete portable, cierra
la aplicación y elimina la carpeta extraída.
