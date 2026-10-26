# Proceso de release

El orden correcto es **release primero, landing después**.

## Pasos

1. **Mergear a `main`**
   Todos los cambios que vayan a incluirse deben estar ya en `main`.

2. **Actualizar versiones**
   Subir la versión en:
   - `package.json` (raíz)
   - `apps/desktop/src-tauri/tauri.conf.json`
   - `Cargo.toml` (workspace)
   - `Cargo.lock` si aplica

3. **Crear y pushear el tag**
   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
   Esto dispara el workflow de GitHub Actions que compila y publica el release.

4. **Esperar a que el CI publique el release**
   Ir a [Releases](https://github.com/SebaAguiar/evo-linux-control-suite/releases) y esperar a que aparezca `vX.Y.Z` con todos los assets (`.pkg.tar.zst`, `.AppImage`, `.deb`, `latest.json`, firmas, etc.).

5. **Verificar `releases/latest`**
   Comprobar que [releases/latest](https://github.com/SebaAguiar/evo-linux-control-suite/releases/latest) apunta al nuevo release con los archivos subidos.

   > El comando de instalación de la landing usa `/releases/latest/download/...`. GitHub resuelve `latest` al release más reciente **publicado**, no al texto que se vea en la web. Por tanto, **en cuanto esté publicado con los assets, el comando ya funcionará aunque la landing siga mostrando la versión antigua.**

6. **Actualizar la landing (solo cosmético)**
   Si se quiere que el texto `Descargar vX.Y.Z` coincida con la versión real, actualizar `apps/landing/src/pages/index.astro`, `apps/landing/public/downloads/install-arch.sh` y `apps/landing/public/downloads/SHA256SUMS`. Hacer commit y push a `main`.  
   **Nunca hacer este paso antes de tener el release publicado.**
