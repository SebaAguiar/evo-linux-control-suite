use std::env;

use clap::Parser;
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

#[derive(Parser, Debug)]
#[command(name = "evo-desktop")]
#[command(about = "EVO Linux Control Suite Desktop", long_about = None)]
struct Args {
    /// Run in panel/applet mode (for Wayland bars like Niri/Noctalia)
    #[arg(long)]
    panel: bool,
    /// Anchor panel to a specific edge: top, bottom, left, right
    #[arg(long, default_value = "top")]
    panel_edge: String,
    /// Horizontal offset for the panel
    #[arg(long, default_value = "0")]
    panel_x: i32,
    /// Vertical offset for the panel
    #[arg(long, default_value = "0")]
    panel_y: i32,
    /// Exclusive zone reserved by the panel (0 = overlay, no space reserved)
    #[arg(long, default_value = "0")]
    panel_exclusive: i32,
}

#[cfg(target_os = "linux")]
fn apply_layer_shell(
    window: &tauri::WebviewWindow,
    edge_name: &str,
    x: i32,
    y: i32,
    exclusive: i32,
) -> tauri::Result<()> {
    use gtk_layer_shell::{Edge, Layer, LayerShell};

    let gtk_window = window.gtk_window()?;
    gtk_window.init_layer_shell();
    gtk_window.set_namespace("evo-panel");
    gtk_window.set_layer(Layer::Top);

    match edge_name.to_ascii_lowercase().as_str() {
        "bottom" => {
            gtk_window.set_anchor(Edge::Bottom, true);
            gtk_window.set_anchor(Edge::Left, true);
            gtk_window.set_layer_shell_margin(Edge::Bottom, y);
            gtk_window.set_layer_shell_margin(Edge::Left, x);
        }
        "left" => {
            gtk_window.set_anchor(Edge::Left, true);
            gtk_window.set_anchor(Edge::Top, true);
            gtk_window.set_layer_shell_margin(Edge::Left, x);
            gtk_window.set_layer_shell_margin(Edge::Top, y);
        }
        "right" => {
            gtk_window.set_anchor(Edge::Right, true);
            gtk_window.set_anchor(Edge::Top, true);
            gtk_window.set_layer_shell_margin(Edge::Right, x);
            gtk_window.set_layer_shell_margin(Edge::Top, y);
        }
        _ => {
            gtk_window.set_anchor(Edge::Top, true);
            gtk_window.set_anchor(Edge::Left, true);
            gtk_window.set_layer_shell_margin(Edge::Top, y);
            gtk_window.set_layer_shell_margin(Edge::Left, x);
        }
    }

    gtk_window.set_exclusive_zone(exclusive);
    Ok(())
}

fn toggle_panel(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("panel") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

fn toggle_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args = Args::parse();
    let panel_mode =
        args.panel || env::var_os("EVO_PANEL_MODE").is_some() || env::var_os("EVO_PANEL").is_some();
    let panel_edge = args.panel_edge.clone();
    let panel_x = args.panel_x;
    let panel_y = args.panel_y;
    let panel_exclusive = args.panel_exclusive;

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(move |app, _args, _cwd| {
            if panel_mode {
                toggle_panel(app);
            } else if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            if panel_mode {
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.hide();
                }
                if let Some(panel) = app.get_webview_window("panel") {
                    let _ = panel.set_decorations(false);
                    let _ = panel.set_skip_taskbar(true);
                    let _ = panel.set_always_on_top(true);
                    let _ = panel.set_resizable(false);

                    #[cfg(target_os = "linux")]
                    if let Err(err) = apply_layer_shell(
                        &panel,
                        &panel_edge,
                        panel_x,
                        panel_y,
                        panel_exclusive,
                    ) {
                        eprintln!(
                            "evo-desktop: layer-shell no disponible ({err}); usando ventana flotante"
                        );
                    }

                    let _ = panel.show();
                }
            } else if let Some(panel) = app.get_webview_window("panel") {
                let _ = panel.hide();
            }

            let toggle_item =
                MenuItem::with_id(app, "toggle", "Mostrar / Ocultar", true, None::<&str>)?;
            let panel_item =
                MenuItem::with_id(app, "toggle_panel", "Alternar Panel", true, None::<&str>)?;
            let quit_item =
                MenuItem::with_id(app, "quit", "Salir de EVO Control", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle_item, &panel_item, &quit_item])?;

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false);

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            let _tray = tray_builder
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "toggle" => {
                        if panel_mode {
                            toggle_panel(app);
                        } else {
                            toggle_main(app);
                        }
                    }
                    "toggle_panel" => toggle_panel(app),
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(move |tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if panel_mode {
                            toggle_panel(app);
                        } else {
                            toggle_main(app);
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
