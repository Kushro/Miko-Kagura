# Miko-Kagura

[English](README.md) | **Español**

Miko-Kagura es el complemento de escritorio de [Miko](https://github.com/Kushro/miko).
Ejecuta el escalado remoto de imágenes de Miko en la GPU de tu computadora.
Usa el panel para elegir modelos, ajustar el procesamiento y consultar las
solicitudes del lector.

## Elige una descarga

Abre [Releases](https://github.com/Kushro/Miko-Kagura/releases) y elige un paquete
para **Windows x64**. Cada release empaquetada ofrece estas cuatro variantes:

| Sufijo del archivo | Incluye motores y modelos | Uso recomendado |
| --- | --- | --- |
| `setup-full.exe` | Sí | Primera instalación (recomendado) |
| `portable-full.zip` | Sí | Ejecutar sin instalar |
| `setup-lite.exe` | No | Instalar o actualizar con motores existentes |
| `portable-lite.zip` | No | Usar tus propios motores sin instalar |

Full incluye motores y modelos de Waifu2x, Real-CUGAN y Real-ESRGAN descargados
desde sus fuentes oficiales. Lite incluye la misma aplicación y funcionalidades.
Los archivos **Source code** de GitHub contienen código fuente, no un paquete ejecutable.

## Requisitos

- Windows x64 y una GPU con un controlador actualizado compatible con Vulkan.
- Microsoft Edge WebView2 Runtime. El instalador lo instala si falta;
  ese paso requiere Internet. Para la versión portable, instálalo por separado.
- Miko y la computadora accesibles desde la misma red de confianza.

## Primeros pasos

1. Instala **Setup Full**, o extrae **Portable Full** y abre `miko-kagura.exe`.
2. Conserva el modelo predeterminado `realcugan-se` para la primera solicitud.
3. Consulta la IP local de la computadora, por ejemplo `192.168.1.100`.
4. Configura `http://192.168.1.100:8282` como dirección del servidor de escalado remoto en Miko.
5. Abre `http://192.168.1.100:8282/health`. `upscaler_ready` debería ser `true`.
6. Activa el escalado remoto en Miko y abre una imagen.

La API no tiene autenticación. Usa una red de confianza y permite el puerto
seleccionado en el firewall para esa red. Consulta [Instalación](docs/es/installation.md)
para configurar Lite, actualizar, verificar descargas y desinstalar.

## Documentación

| Objetivo | Guía |
| --- | --- |
| Instalar, actualizar o elegir motores | [Instalación](docs/es/installation.md) |
| Elegir modelos y cambiar preferencias | [Configuración](docs/es/configuration.md) |
| Resolver problemas de conexión o procesamiento | [Solución de problemas](docs/es/troubleshooting.md) |
| Integrar otro cliente | [API HTTP](docs/es/api.md) |
| Compilar y entender la aplicación | [Desarrollo](docs/es/development.md) |
| Generar y publicar paquetes | [Publicación](docs/es/releasing.md) |
| Consultar las dependencias incluidas | [Componentes de terceros](docs/es/third-party-components.md) |

Portable significa **sin instalación**. Las preferencias se guardan en
`%APPDATA%\Miko-Kagura\settings.json`, incluso al ejecutar un paquete portable.

## Licencia

El código original del proyecto usa [PolyForm Noncommercial 1.0.0](LICENSE):
`Required Notice: Copyright (c) 2026 Octavio`. Es software de código disponible
para los usos no comerciales permitidos. Los componentes de terceros conservan
sus propias licencias y permisos. Consulta los [avisos de terceros](THIRD_PARTY_NOTICES.md).
