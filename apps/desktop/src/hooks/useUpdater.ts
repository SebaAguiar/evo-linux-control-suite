import { useState, useEffect, useCallback, useRef } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export interface UpdateInfo {
  version: string;
  notes?: string;
  date?: string;
}

const CHECK_INTERVAL_MS = 60 * 60 * 1000; // Cada 1 hora

export function useUpdater() {
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const activeUpdateRef = useRef<Update | null>(null);

  const isTauri =
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

  const checkForUpdates = useCallback(async () => {
    if (!isTauri) return;

    setChecking(true);
    setError(null);
    try {
      const update = await check();
      if (update?.available) {
        activeUpdateRef.current = update;
        setUpdateInfo({
          version: update.version,
          notes: update.body,
          date: update.date,
        });
      } else {
        activeUpdateRef.current = null;
        setUpdateInfo(null);
      }
    } catch (err: unknown) {
      console.warn("Error comprobando actualizaciones:", err);
      // No molestamos al usuario con errores silenciosos de red/offline
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setChecking(false);
    }
  }, [isTauri]);

  const downloadAndInstall = useCallback(async () => {
    if (!activeUpdateRef.current) {
      await checkForUpdates();
      if (!activeUpdateRef.current) return;
    }

    setDownloading(true);
    setProgress(0);
    setError(null);

    try {
      let downloaded = 0;
      let contentLength = 0;

      await activeUpdateRef.current.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            contentLength = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            if (contentLength > 0) {
              const pct = Math.round((downloaded / contentLength) * 100);
              setProgress(pct);
            }
            break;
          case "Finished":
            setProgress(100);
            break;
        }
      });

      // En Linux/macOS, relaunch reinicia la app con el nuevo binario
      await relaunch();
    } catch (err: unknown) {
      console.error("Fallo al descargar o instalar la actualización:", err);
      setError(err instanceof Error ? err.message : String(err));
      setDownloading(false);
    }
  }, [checkForUpdates]);

  useEffect(() => {
    if (!isTauri) return;

    // Comprobación inicial al abrir la app
    void checkForUpdates();

    // Comprobación periódica cada hora
    const timer = setInterval(() => {
      void checkForUpdates();
    }, CHECK_INTERVAL_MS);

    return () => clearInterval(timer);
  }, [isTauri, checkForUpdates]);

  return {
    isTauri,
    updateInfo,
    checking,
    downloading,
    progress,
    error,
    checkForUpdates,
    downloadAndInstall,
  };
}
