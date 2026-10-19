# Marca — EVO Linux Control Suite

Logotipo propio inspirado en el ADN visual de EVO (Audient), sin copiarlo: misma familia geométrica monolineal y mismo lockup "wordmark + descriptor en versalitas espaciadas", pero con un glifo propio y el acento de la marca.

## Concepto

El glifo del icono se lee a tres niveles:

1. **La "e" de EVO** — anillo circular con crossbar horizontal (la "e" geométrica del logo original).
2. **El dial / LED ring del hardware** — el anillo es la perilla central del EVO 4; la muesca superior es el marcador de posición del dial.
3. **El plano de control** — el crossbar en `--accent` (#818CF8) representa el nivel/señal: el software que controla el hardware.

## Archivos

| Archivo | Uso |
| --- | --- |
| `evolinux-icon.svg` | Icono de app, favicon, tray (tema oscuro) |
| `evolinux-icon-light.svg` | Icono para superficies claras |
| `evolinux-lockup-stacked.svg` | Lockup vertical: "evo" + `LINUX CONTROL SUITE` (títulos, splash, README) |
| `evolinux-lockup-horizontal.svg` | Lockup horizontal: icono + wordmark (barras de ventana, encabezados) |
| `preview.html` | Hoja de revisión: todos los variantes + escala de tamaños + paleta |

## Paleta

| Token | Hex | Uso en el logo |
| --- | --- | --- |
| `--background` | `#111827` | Tile del icono |
| `--text-primary` | `#F9FAFB` | Anillo y wordmark "evo" |
| `--text-muted` | `#9CA3AF` | Descriptor `LINUX CONTROL SUITE` |
| `--accent` | `#818CF8` | Crossbar del glifo (único punto de color) |

## Notas de uso

- El descriptor del lockup está compuesto con **Inter** (fallback: Helvetica/Arial) en versalitas con tracking de 0.3 em. Antes de exportar arte final para impresión, convertir el texto a trazados (`text → path`).
- El wordmark "evo" ya es 100 % trazados vectoriales (monolineal geométrico: grosor 15/100 de la altura de x), por lo que es seguro escalar sin fuentes.
- Tamaño mínimo del icono: 24 px (la muesca empieza a fundirse por debajo; a 32 px se lee bien).
- Para generar los PNG de Tauri (`src-tauri/icons/`) exportar el SVG a 32/128/256/512 y al `.ico` correspondiente.
