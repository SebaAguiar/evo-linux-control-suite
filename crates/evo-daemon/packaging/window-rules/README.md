# Reglas de ventana flotante para EVO Control en Tiling Window Managers

En entornos de escritorio tradicionales (**GNOME, KDE Plasma, XFCE, Cinnamon, MATE, LXQt**), la aplicación abre automáticamente como ventana flotante centrada de 620x360 sin necesidad de configuración adicional.

En gestores de ventana tipo **Tiling / Tiling Wayland Compositors** (i3, Sway, Hyprland, Niri, River, bspwm), las ventanas se acomodan en mosaico por defecto. Usá la regla correspondiente a tu entorno:

---

### Hyprland (`~/.config/hypr/hyprland.conf` o `conf.d/`)
Agregá estas líneas a tu archivo de configuración:
```ini
windowrulev2 = float, class:^(evo-desktop)$
windowrulev2 = size 620 360, class:^(evo-desktop)$
windowrulev2 = center, class:^(evo-desktop)$
```

---

### Sway (`~/.config/sway/config`)
Agregá a tu configuración de Sway:
```ini
for_window [app_id="evo-desktop"] floating enable, resize set 620 360, move position center
```

---

### i3 (`~/.config/i3/config`)
Agregá a tu configuración de i3:
```ini
for_window [class="(?i)evo-desktop"] floating enable, resize set 620 360, move position center
```

---

### Niri (`~/.config/niri/config.kdl`)
Agregá al bloque de `window-rule`:
```kdl
window-rule {
    match app-id="evo-desktop"
    open-floating true
    default-column-width { fixed 620; }
    default-window-height { fixed 360; }
}
```

---

### River (`~/.config/river/init`)
```bash
riverctl rule-add -app-id evo-desktop float
```

---

### bspwm (`~/.config/bspwm/bspwmrc`)
```bash
bspc rule -a evo-desktop state=floating
```
