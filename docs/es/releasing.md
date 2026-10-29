# Generar y publicar una release

[English](../en/releasing.md) | **Español** | [Inicio](../../README.es.md)

El workflow genera Portable Lite/Full y Setup Lite/Full para Windows x64, además
de tarballs Portable Lite/Full, un AppImage Full y un .deb Full para Linux x64, y
los adjunta junto con un único `SHA256SUMS.txt` a la release que lo activó.
Todos los jobs usan los runners que elige `runner.yml` (Avrea por defecto).

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
se detiene al detectarlas. Si necesitas inmutabilidad, completa primero un borrador
(consulta [Adjuntar a un borrador antes de publicar](#adjuntar-a-un-borrador-antes-de-publicar)). Consulta
[eventos de release de GitHub](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#release)
y [releases inmutables](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).

### Adjuntar a un borrador antes de publicar

Para revisar las descargas antes de que alguien vea la release, créala como
borrador y ejecuta el workflow con `attach` marcado:

```powershell
gh release create v0.1.0 --repo Kushro/Miko-Kagura --verify-tag --draft --title "Miko-Kagura v0.1.0" --generate-notes
gh workflow run release.yml --repo Kushro/Miko-Kagura -f tag=v0.1.0 -f attach=true
```

La ejecución compila y prueba el commit de la tag, no la referencia del workflow,
y sube los archivos al único borrador de esa tag; rechaza una release publicada.
Cuando después publiques el borrador, `release: published` encuentra todos los
paquetes ya adjuntos y omite la compilación y la subida, así que se publican los
mismos archivos que revisaste.

## Paquetes Linux y el AUR

El job `linux` usa el runner Ubuntu 24.04 de Avrea, así que los paquetes requieren
glibc 2.39 o posterior. Para reproducirlo en Ubuntu 24.04 con las dependencias
del workflow, Dioxus CLI 0.7.9 y bun:

```bash
bash scripts/build-release-linux.sh v0.1.0
bash scripts/test-linux-packages.sh v0.1.0
```

Los paquetes quedan en `dist/v0.1.0-linux`. El build descarga los motores fijados
en `packaging/upscalers-linux-x64.lock.json` y versiones fijadas de `appimagetool`
y del runtime de AppImage. El .deb obtiene su `Depends` de `dpkg-shlibdeps`. El
script de pruebas necesita Docker (también funciona `DOCKER=podman`). Instala el
.deb y ejecuta el AppImage en Ubuntu 24.04, compila e instala el paquete del AUR
en Arch, comprueba las bibliotecas y espera a que `/health` detecte los motores.

El AppImage y el .deb incluyen iconos PNG, metadata AppStream
(`packaging/linux/miko-kagura.metainfo.xml`, completada con la versión y la fecha
de la release) y, en el AppImage, `X-AppImage-Version`, para que los gestores de
AppImage muestren el icono y la versión. Los PNG de `packaging/linux/icons` están
versionados; si editás `packaging/linux/miko-kagura.svg`, regeneralos con
`bash scripts/render-linux-icons.sh` (requiere `rsvg-convert`).

El workflow conserva el `PKGBUILD` y el `.SRCINFO` probados como artefacto
`aur-miko-kagura-bin-<tag>`. No son archivos de la release. Tras publicarla,
copia ambos archivos a tu clon de `ssh://aur@aur.archlinux.org/miko-kagura-bin.git`,
haz commit y push. Para regenerarlos en Arch desde un checkout de la tag:

```bash
scripts/update-aur.sh v0.1.0 SHA256SUMS.txt aur-out
cd aur-out && makepkg --printsrcinfo > .SRCINFO
```

Publica en el AUR solo versiones estables. Si publicas una prerelease,
`0.2.0-beta.1` pasa a `pkgver=0.2.0beta.1`, que pacman ordena antes de `0.2.0`.

## Pruebas manuales y reintentos de subida

Usa **Actions → Release packages → Run workflow** e indica una tag de
versión para compilar sin publicar. Se compila la referencia seleccionada del
workflow (salvo con `attach` marcado), no una tag elegida por ese campo de texto. La versión debe coincidir
con sus manifiestos. Los paquetes y el `SHA256SUMS.txt` de cada plataforma se
conservan como artefacto de Actions durante 14 días.

La subida omite archivos existentes idénticos y falla ante nombres en conflicto.
Nunca reemplaza descargas silenciosamente. Si una subida falla parcialmente,
descarga los artefactos originales de Actions en `dist/<tag>` (Windows) y
`dist/<tag>-linux` (Linux) y reintenta con esos archivos:

```powershell
$env:GH_REPO = 'Kushro/Miko-Kagura'
$releaseId = gh release view v0.1.0 --json databaseId --jq .databaseId
pwsh -File scripts/publish-release.ps1 -Tag v0.1.0 -ReleaseId $releaseId
```

Agrega `-Draft` si la release todavía es un borrador.

Una recompilación puede cambiar las fechas internas de ZIP e instaladores, por
lo que un reintento completo puede detectar un conflicto válido. Inspecciona los
archivos existentes antes de eliminarlos explícitamente.
