# Landing de EVO Linux Control Suite

Landing estática en [Astro](https://astro.build) publicada en GitHub Pages.

El diseño está portado 1:1 desde el proyecto **Figma Make** *Landing page para EVO
Linux* (`apps/landing/src/styles/design.css` es el CSS original exportado por el
diseño). Los mockups de la app que traía el diseño se reemplazaron por capturas
reales del software, tomadas con Playwright:

| Captura | Origen |
| --- | --- |
| `src/assets/screenshots/deck.png` | Deck de escritorio (Tauri) a 620×360 @2x |
| `src/assets/screenshots/controller-mobile.png` | Controlador web del daemon a 390×844 @2x |
| `src/assets/screenshots/controller-wide.png` | Controlador web en viewport ancho (sin usar en la página) |

## Desarrollo

```bash
cd apps/landing
pnpm install
pnpm dev        # http://localhost:4321/evo-linux-control-suite/
pnpm build      # genera dist/
pnpm preview    # sirve dist/ en http://localhost:4321/evo-linux-control-suite/
```

## Deploy

El workflow [`.github/workflows/pages.yml`](../../.github/workflows/pages.yml)
compila y publica `apps/landing/dist` en cada push a `main` que toque la landing
(o manualmente desde *Actions → Deploy landing*).

Requisitos, una sola vez en el repositorio:

1. **Settings → Pages → Build and deployment → Source: GitHub Actions**.
2. Mergear este cambio a `main` (o correr el workflow a mano desde `main`).

La URL queda en `https://sebaaguiar.github.io/evo-linux-control-suite/`.

`astro.config.mjs` define:

- `site: "https://sebaaguiar.github.io"`
- `base: "/evo-linux-control-suite"` (Pages de proyecto)

Con un dominio propio, cambiá `site` por el dominio y dejá `base: "/"`.

## Estructura

```
src/
├── assets/screenshots/   # capturas reales del software
├── pages/index.astro     # página completa (markup + interacciones)
└── styles/
    ├── design.css        # CSS original del diseño de Figma Make
    └── landing.css       # ajustes: capturas reales, tabs, responsive
public/favicon.svg
```

Interacciones incluidas (JS sin dependencias): cambio de paleta Arsian Dark /
EVO Green, aparición al scrollear, menú móvil, acordeón de FAQ, pestañas de
instalación y botón de copiar comando.
