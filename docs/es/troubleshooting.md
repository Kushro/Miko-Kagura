# Resolver problemas de Miko-Kagura

[English](../en/troubleshooting.md) | **Español** | [Inicio](../../README.es.md)

Comienza por la pestaña **Errors** y el endpoint `/health` para distinguir
problemas de conexión, motores ausentes y fallos de inferencia.

## Miko no puede conectarse

1. Mantén Miko-Kagura abierto y comprueba el puerto de escucha.
2. En la computadora, abre `http://127.0.0.1:8282/health`.
3. En el teléfono, usa la IP local de la computadora en lugar de `localhost`.
4. Revisa el firewall y el aislamiento de clientes de la red Wi-Fi.

Si otro proceso usa el puerto, inicia con `--port 8283` y cambia la dirección
en Miko para que coincida. Consulta [Instalación](installation.md).

## El servidor indica que no está listo

Una respuesta `upscaler_ready: false` significa que el modelo o ejecutable actual
no está disponible. Con Lite, configura **Binary Directory**. Con Full, comprueba
que `binaries/` se haya extraído completamente. Una ruta absoluta guardada puede
apuntar a una instalación antigua; selecciona la carpeta actual.

## Falla Vulkan o la carga de modelos

Actualiza el controlador de GPU y verifica su soporte Vulkan. Prueba
`--gpu-id 0` si la selección automática elige un dispositivo inadecuado.
Comprueba que la carpeta de modelos contenga archivos `.param` y `.bin`.
Usa una combinación de modelo, escala y ruido compatible con ese motor.

El nombre de GPU mostrado puede proceder del adaptador integrado de Windows,
aunque la inferencia use otra GPU. La salida de inferencia de NCNN indica el
dispositivo relevante; el nombre del panel no determina por sí solo la selección.

## La ventana no tiene estilos o no se abre

Extrae todo el paquete, incluido `assets/`. Al compilar desde el código fuente,
usa `dx build` o `dx serve` para recopilar los estilos. `cargo run` por sí solo
no prepara esos recursos. Instala WebView2 Runtime si la ventana no se abre.

## Las imágenes están desordenadas o las comparaciones no tienen etiquetas

En solicitudes por URL, activa el plugin de la fuente y conserva el fragmento de
URL. Revisa **Errors** para detectar reparaciones ausentes o fallidas. En cargas
directas, el lector se encarga de decodificar. Consulta [Configuración](configuration.md).

Las etiquetas de comparación requieren una fuente TrueType disponible, como
`C:\Windows\Fonts\arial.ttf`. Sin una fuente, la imagen comparativa no tiene etiquetas.

## Fallan la instalación o las actualizaciones

Cierra la aplicación antes de instalar o desinstalar. Si falla la instalación de
WebView2, instala el runtime desde Microsoft y ejecuta Setup nuevamente.
Ese paso puede requerir Internet incluso con la edición Full.

Si el problema continúa, incluye nombre del paquete, versión de Windows,
GPU/controlador, modelo y errores relevantes en un
[issue](https://github.com/Kushro/Miko-Kagura/issues). Elimina URLs o cabeceras privadas.
