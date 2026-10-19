import { useState, useEffect, useCallback } from "react";

export type Theme = "brand" | "evo";

const THEME_STORAGE_KEY = "evo_ui_theme_v2";

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(() => {
    if (typeof window !== "undefined") {
      try {
        const saved = localStorage.getItem(THEME_STORAGE_KEY) as Theme | null;
        if (saved === "brand" || saved === "evo") return saved;
      } catch {
        // Fallback
      }
    }
    return "brand"; // Arsian por defecto
  });

  useEffect(() => {
    if (typeof document !== "undefined") {
      document.documentElement.setAttribute("data-theme", theme);
      if (document.body) {
        document.body.setAttribute("data-theme", theme);
      }
    }
    try {
      localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch {
      // Ignorar en entornos restringidos
    }
  }, [theme]);

  const toggleTheme = useCallback(() => {
    setTheme((prev) => (prev === "brand" ? "evo" : "brand"));
  }, []);

  return { theme, setTheme, toggleTheme };
}
