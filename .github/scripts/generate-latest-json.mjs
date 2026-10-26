#!/usr/bin/env node
/**
 * Genera el manifiesto estático (latest.json) del auto-updater de Tauri v2 a
 * partir de los artefactos firmados del release.
 *
 * El plugin busca las claves en este orden: `linux-x86_64-{appimage,deb,rpm}` y
 * después `linux-x86_64`. Solo se publican las claves específicas: así una
 * instalación nativa fuera de esos formatos (por ejemplo el paquete de Arch, que
 * deja el binario en /usr/bin) no recibe una actualización que no puede aplicar,
 * y se actualiza por su gestor de paquetes.
 *
 * Env:
 *   ARTIFACTS_DIR  directorio con los instaladores y sus .sig
 *   TAG            tag del release, p. ej. v1.0.1
 *   REPO           repositorio que aloja el release, p. ej. usuario/repo
 *   NOTES_FILE     notas del release (por defecto CHANGELOG.md)
 *
 * Los paquetes se reconocen por su forma (`<nombre>_<versión>_<arch>.<formato>`)
 * y no por el nombre del producto: tauri-bundler genera `<productName>_...`, que
 * puede llevar espacios según la versión del bundler.
 */
import { readdirSync, readFileSync, existsSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const dir = process.env.ARTIFACTS_DIR;
const tag = process.env.TAG;
const repo = process.env.REPO;
const notesFile = process.env.NOTES_FILE ?? 'CHANGELOG.md';

if (!dir || !tag || !repo) {
  console.error(
    'Usage: ARTIFACTS_DIR=<dir> TAG=<vX.Y.Z> REPO=<owner/repo> node generate-latest-json.mjs',
  );
  process.exit(1);
}

const version = tag.replace(/^v/, '');

/** `..._1.0.1_amd64.deb`, `..._1.0.1_amd64.AppImage.tar.gz`, `..._1.0.1_amd64.rpm`. */
const ASSET_PATTERN =
  /_(\d+\.\d+\.\d+(?:[-+][^_/]*)?)_([A-Za-z0-9_]+)\.(?:AppImage|deb|rpm)(?:\.tar\.gz)?$/;

const allFiles = readdirSync(dir).sort();
const files = allFiles.filter((file) => !file.endsWith('.sig') && ASSET_PATTERN.test(file));

if (files.length === 0) {
  console.error(
    `No hay paquetes instalables (<nombre>_<versión>_<arch>.<formato>) en ${dir}; ` +
      'no se puede generar latest.json.',
  );
  process.exit(1);
}

const downloadUrl = (asset) => `https://github.com/${repo}/releases/download/${tag}/${asset}`;

const readSignature = (asset) => {
  const sigFile = `${asset}.sig`;
  if (!allFiles.includes(sigFile)) {
    throw new Error(
      `falta la firma de ${asset} (esperaba ${sigFile}); revisá TAURI_SIGNING_PRIVATE_KEY`,
    );
  }
  const signature = readFileSync(join(dir, sigFile), 'utf8').trim();
  if (signature === '') {
    throw new Error(`la firma de ${asset} está vacía`);
  }
  return signature;
};

/** Versión codificada en el nombre del paquete: `..._1.0.1_amd64.deb`. */
const versionOf = (asset) => ASSET_PATTERN.exec(asset)?.[1] ?? '';

const pick = (predicate) => files.find(predicate);

const platforms = {};
const addPlatform = (key, asset) => {
  const assetVersion = versionOf(asset);
  if (assetVersion !== version) {
    throw new Error(
      `${asset} está versionado como ${assetVersion} pero el tag es ${tag}: ` +
        'el updater lo volvería a ofrecer en bucle. Actualizá la versión del paquete o el tag.',
    );
  }
  platforms[key] = {
    url: downloadUrl(asset),
    signature: readSignature(asset),
  };
};

try {
  // Tauri v2 publica el AppImage como `.AppImage.tar.gz` firmado; el `.AppImage`
  // suelto también se acepta para versiones del bundler que no lo comprimen.
  const appImage =
    pick((file) => file.endsWith('.AppImage.tar.gz')) ?? pick((file) => file.endsWith('.AppImage'));
  if (appImage) {
    addPlatform('linux-x86_64-appimage', appImage);
  }

  const deb = pick((file) => file.endsWith('.deb'));
  if (deb) {
    addPlatform('linux-x86_64-deb', deb);
  }

  const rpm = pick((file) => file.endsWith('.rpm'));
  if (rpm) {
    addPlatform('linux-x86_64-rpm', rpm);
  }

  if (Object.keys(platforms).length === 0) {
    throw new Error(
      'Ningún paquete sirve para el updater ' +
        '(se esperaba .AppImage[.tar.gz], .deb o .rpm con su .sig).',
    );
  }

  const notes = existsSync(notesFile)
    ? readFileSync(notesFile, 'utf8').slice(0, 2000)
    : `Release ${tag}`;

  const manifest = {
    version,
    notes,
    pub_date: new Date().toISOString(),
    platforms,
  };

  const outPath = join(dir, 'latest.json');
  writeFileSync(outPath, `${JSON.stringify(manifest, null, 2)}\n`);

  console.log(`latest.json escrito para ${manifest.version}`);
  for (const [key, entry] of Object.entries(platforms)) {
    console.log(`  ${key} -> ${entry.url}`);
  }
} catch (error) {
  console.error(`Error: ${error.message}`);
  process.exit(1);
}
