# Generar y publicar una release

[English](../en/releasing.md) | **Español** | [Inicio](../../README.es.md)

El workflow genera Portable Lite/Full y Setup Lite/Full para Windows x64,
y los adjunta junto con `SHA256SUMS.txt` a la release que lo activó.

## Preparar una versión

1. Actualiza `Cargo.toml` y `package.json` con la misma versión SemVer.
2. Actualiza `Cargo.lock` con `cargo check` y revisa los cambios de dependencias.
3. Actualiza ambos idiomas de la documentación y el manifiesto de motores si corresponde.
4. Confirma y sube los cambios, incluido el workflow, antes de crear la tag.

Por ejemplo, `0.1.0` usa `v0.1.0`; `0.2.0-beta.1` usa `v0.2.0-beta.1`.
Las tags deben empezar con `v` minúscula y coincidir con la versión del paquete.
Los componentes numéricos de Windows deben ser como máximo `65535`.
Los sufijos de prerelease/build se conservan en nombres y textos; la versión
numérica NSIS es `major.minor.patch.0`.

## Compilar localmente

Prepara los [requisitos de desarrollo](development.md) y ejecuta PowerShell 7:

```powershell
pwsh -File scripts/build-release.ps1 -Tag v0.1.0
```

El script ejecuta pruebas, compila Tailwind y Dioxus, descarga motores con hashes
fijados y prepara Lite/Full e instaladores NSIS. Las descargas se guardan en
`target/download-cache`. El trabajo generado queda en `target/release-packaging`
y los paquetes en `dist/v0.1.0`. Solo se reinician esas carpetas generadas.
`-SkipTests` permite repetir el empaquetado local después de aprobar las pruebas.

La plantilla NSIS personalizada usa definiciones generadas y la variable
`MIKO_INSTALLER_DEFINES`. Usa el script de empaquetado en lugar de invocar
`dx bundle` directamente. Dioxus descarga las
herramientas NSIS y el instalador inicial de WebView2 durante el empaquetado.

## Validar los paquetes

```powershell
bun run docs:check
pwsh -File scripts/test-installers.ps1 -Tag v0.1.0
```

La prueba de instalación usa una carpeta temporal y comprueba Lite, Full,
reinstalación, ambos cambios de edición y conservación de archivos del usuario.
No se ejecuta si ya existe una instalación o accesos directos de Miko-Kagura
para ese usuario. Usa preferiblemente un runner Windows desechable.
Las preferencias del usuario no se eliminan.

Antes de dar una release por validada, abre Full fuera del repositorio,
comprueba los estilos, consulta `/health` y escala una imagen real en una GPU
compatible con Vulkan. Los runners alojados no verifican compatibilidad de GPU.

## Publicar en GitHub

Crea la tag y publica su release desde GitHub o con tu CLI autenticada:

```powershell
git tag v0.1.0
git push origin v0.1.0
gh release create v0.1.0 --repo Kushro/Miko-Kagura --verify-tag --title "Miko-Kagura v0.1.0" --generate-notes
```

`release: published` cubre releases estables y prereleases. Un borrador o un push
de tag por sí solos no activan el empaquetado. El workflow omite releases sin prefijo
`v`, obtiene el commit del evento y valida la versión antes de compilar.
Las pruebas y el empaquetado corren en jobs paralelos; un job final de publicación
sube los archivos solo si ambos terminan bien. La release comienza sin descargas compiladas.

El `GITHUB_TOKEN` automático aporta `contents: write` solo al job de publicación; no hace falta un token
personal para subir archivos. Crear la release desde otro workflow con
`GITHUB_TOKEN` normalmente no activa este flujo; usa un token de GitHub App
apropiado o ejecuta los pasos de empaquetado desde ese mismo workflow.

Las **releases inmutables publicadas no admiten nuevos archivos**. Este workflow
se detiene al detectarlas. Si necesitas inmutabilidad, cambia el proceso para
compilar, subir a un borrador y después publicar. Consulta
[eventos de release de GitHub](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#release)
y [releases inmutables](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).

## Pruebas manuales y reintentos de subida

Usa **Actions → Release Windows packages → Run workflow** e indica una tag de
versión para compilar sin publicar. Se compila la referencia seleccionada del
workflow, no una tag elegida por ese campo de texto. La versión debe coincidir
con sus manifiestos. Los cinco archivos se conservan como artefacto de Actions
durante 14 días.

La subida omite archivos existentes idénticos y falla ante nombres en conflicto.
Nunca reemplaza descargas silenciosamente. Si una subida falla parcialmente,
descarga el artefacto original de Actions en `dist/<tag>` y reintenta con esos archivos:

```powershell
$env:GH_REPO = 'Kushro/Miko-Kagura'
$releaseId = gh release view v0.1.0 --json databaseId --jq .databaseId
pwsh -File scripts/publish-release.ps1 -Tag v0.1.0 -ReleaseId $releaseId
```

Una recompilación puede cambiar las fechas internas de ZIP e instaladores, por
lo que un reintento completo puede detectar un conflicto válido. Inspecciona los
archivos existentes antes de eliminarlos explícitamente.
