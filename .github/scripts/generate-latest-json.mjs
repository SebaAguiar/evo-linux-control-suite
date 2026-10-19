#!/usr/bin/env node
/**
 * Generates the Tauri v2 static updater manifest (latest.json) from staged release artifacts.
 *
 * Env:
 *   ARTIFACTS_DIR  dir containing staged installers + .sig files
 *   TAG            tag name of the release, e.g. v0.1.1
 *   REPO           GitHub repo hosting the release, e.g. SebaAguiar/evo-linux-control-suite
 *   NOTES_FILE     optional path to release notes (defaults to CHANGELOG.md)
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

const files = readdirSync(dir).sort();
if (files.length === 0) {
  console.error(`No artifacts found in ${dir}`);
  process.exit(1);
}

const pick = (suffix) => files.find((f) => f.endsWith(suffix));

const readSignature = (asset) => {
  const sigFile = `${asset}.sig`;
  if (!files.includes(sigFile)) {
    throw new Error(`missing signature for ${asset} (expected ${sigFile})`);
  }
  return readFileSync(join(dir, sigFile), 'utf8').trim();
};

const downloadUrl = (asset) =>
  `https://github.com/${repo}/releases/download/${tag}/${asset}`;

const platforms = {};

const linuxAppImage = pick('.AppImage');
const debAsset = pick('.deb');
const tarGzAsset = pick('.tar.gz');

if (linuxAppImage) {
  platforms['linux-x86_64-appimage'] = {
    url: downloadUrl(linuxAppImage),
    signature: readSignature(linuxAppImage),
  };
  platforms['linux-x86_64'] = {
    url: downloadUrl(linuxAppImage),
    signature: readSignature(linuxAppImage),
  };
}

if (debAsset) {
  platforms['linux-x86_64-deb'] = {
    url: downloadUrl(debAsset),
    signature: readSignature(debAsset),
  };
}

if (tarGzAsset && !platforms['linux-x86_64']) {
  platforms['linux-x86_64'] = {
    url: downloadUrl(tarGzAsset),
    signature: readSignature(tarGzAsset),
  };
}

if (Object.keys(platforms).length === 0) {
  console.error(
    'No updater artifacts matched (expected *.AppImage, *.deb, or *.tar.gz with .sig files).',
  );
  process.exit(1);
}

const notes = existsSync(notesFile)
  ? readFileSync(notesFile, 'utf8').slice(0, 2000)
  : `Release ${tag}`;

const manifest = {
  version: tag.replace(/^v/, ''),
  notes,
  pub_date: new Date().toISOString(),
  platforms,
};

const outPath = join(dir, 'latest.json');
writeFileSync(outPath, `${JSON.stringify(manifest, null, 2)}\n`);

console.log(`latest.json written for ${manifest.version}`);
for (const [key, entry] of Object.entries(platforms)) {
  console.log(`  ${key} -> ${entry.url}`);
}
