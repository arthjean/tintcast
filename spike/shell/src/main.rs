#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{collections::VecDeque, time::Duration};

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use gpui::{
    AnyWindowHandle, App, Bounds, Context, FontWeight, Hsla, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use tray_icon::{
    Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem},
};

const BACKGROUND: u32 = 0x17181a;
const SURFACE: u32 = 0x0f1112;
const BORDER: u32 = 0x454b51;
const FOREGROUND: u32 = 0xd9d7d2;
const MUTED: u32 = 0x79818a;
const ACCENT: u32 = 0xe1cc99;
const SUCCESS: u32 = 0xa3cea3;
const ERROR: u32 = 0xda9c94;
const ANSI: [u32; 16] = [
    0x17181a, 0xda9c94, 0xa3cea3, 0xdcca92, 0x8eb1dd, 0xcaa1cc, 0x85d1d0, 0xd9d7d2, 0x454b51,
    0xf3aca3, 0xb2e4b2, 0xf2dd9d, 0x9cc4f8, 0xe1b2e3, 0x8ee6e5, 0xf0eeea,
];

struct Shortcut {
    label: &'static str,
    keys: &'static str,
    hotkey: HotKey,
    status: Result<(), String>,
}

struct Shell {
    window: AnyWindowHandle,
    shortcuts: Vec<Shortcut>,
    tray_status: Result<(), String>,
    quit: Option<MenuId>,
    events: VecDeque<String>,
    _manager: Option<GlobalHotKeyManager>,
    _tray: Option<TrayIcon>,
}

impl Shell {
    fn new(window: AnyWindowHandle, cx: &mut Context<Self>) -> Self {
        let mut shortcuts = vec![
            Shortcut {
                label: "Switcher",
                keys: "Win + Ctrl + Alt + Space",
                hotkey: HotKey::new(
                    Some(Modifiers::META | Modifiers::CONTROL | Modifiers::ALT),
                    Code::Space,
                ),
                status: Err("not registered".into()),
            },
            Shortcut {
                label: "Next background",
                keys: "Win + Ctrl + Alt + B",
                hotkey: HotKey::new(
                    Some(Modifiers::META | Modifiers::CONTROL | Modifiers::ALT),
                    Code::KeyB,
                ),
                status: Err("not registered".into()),
            },
        ];
        let manager = match GlobalHotKeyManager::new() {
            Ok(manager) => {
                for shortcut in &mut shortcuts {
                    shortcut.status = manager.register(shortcut.hotkey).map_err(|e| e.to_string());
                }
                Some(manager)
            }
            Err(error) => {
                for shortcut in &mut shortcuts {
                    shortcut.status = Err(format!("no hotkey manager: {error}"));
                }
                None
            }
        };
        let (tray, quit, tray_status) = match build_tray() {
            Ok((tray, quit)) => (Some(tray), Some(quit), Ok(())),
            Err(error) => (None, None, Err(error)),
        };
        for shortcut in &shortcuts {
            println!("shortcut {}: {:?}", shortcut.keys, shortcut.status);
        }
        println!("tray: {tray_status:?}");
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(30))
                    .await;
                let hotkeys: Vec<GlobalHotKeyEvent> =
                    GlobalHotKeyEvent::receiver().try_iter().collect();
                let menus: Vec<MenuEvent> = MenuEvent::receiver().try_iter().collect();
                let trays: Vec<TrayIconEvent> = TrayIconEvent::receiver().try_iter().collect();
                if hotkeys.is_empty() && menus.is_empty() && trays.is_empty() {
                    continue;
                }
                let alive = this.update(cx, |shell, cx| shell.handle(hotkeys, menus, trays, cx));
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Shell {
            window,
            shortcuts,
            tray_status,
            quit,
            events: VecDeque::new(),
            _manager: manager,
            _tray: tray,
        }
    }

    fn handle(
        &mut self,
        hotkeys: Vec<GlobalHotKeyEvent>,
        menus: Vec<MenuEvent>,
        trays: Vec<TrayIconEvent>,
        cx: &mut Context<Self>,
    ) {
        let mut bring_forward = false;
        for event in hotkeys
            .into_iter()
            .filter(|e| e.state == HotKeyState::Pressed)
        {
            let label = self
                .shortcuts
                .iter()
                .find(|s| s.hotkey.id() == event.id)
                .map_or("unknown", |s| s.label);
            self.log(format!("Shortcut pressed: {label}"));
            bring_forward = true;
        }
        for event in trays {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                self.log("Tray icon clicked".into());
                bring_forward = true;
            }
        }
        for event in menus {
            if Some(&event.id) == self.quit.as_ref() {
                cx.quit();
                return;
            }
        }
        if bring_forward {
            let _ = self
                .window
                .update(cx, |_, window, _| window.activate_window());
        }
        cx.notify();
    }

    fn log(&mut self, line: String) {
        println!("event: {line}");
        self.events.push_front(line);
        self.events.truncate(6);
    }
}

fn build_tray() -> Result<(TrayIcon, MenuId), String> {
    let quit = MenuItem::new("Quit Tintcast", true, None);
    let menu = Menu::new();
    menu.append(&quit).map_err(|e| e.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("Tintcast")
        .with_icon(tray_icon_image()?)
        .with_menu(Box::new(menu))
        .build()
        .map_err(|e| e.to_string())?;
    Ok((tray, quit.id().clone()))
}

fn tray_icon_image() -> Result<Icon, String> {
    let side = 32u32;
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let column = (x * 4 / side) as usize;
            let row = (y * 4 / side) as usize;
            let color = ANSI[(row * 4 + column) % 16];
            let inside = x > 1 && y > 1 && x < side - 2 && y < side - 2;
            rgba.extend([
                (color >> 16) as u8,
                (color >> 8) as u8,
                color as u8,
                if inside { 255 } else { 0 },
            ]);
        }
    }
    Icon::from_rgba(rgba, side, side).map_err(|e| e.to_string())
}

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let status_row = |label: SharedString, keys: SharedString, status: &Result<(), String>| {
            let (mark, text, tint) = match status {
                Ok(()) => ("●", "registered".to_string(), SUCCESS),
                Err(error) => ("●", error.clone(), ERROR),
            };
            div()
                .flex()
                .flex_row()
                .gap(px(12.))
                .py(px(10.))
                .border_b_1()
                .border_color(color(BORDER))
                .child(div().w(px(150.)).text_color(color(FOREGROUND)).child(label))
                .child(div().w(px(210.)).text_color(color(MUTED)).child(keys))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.))
                        .text_color(color(tint))
                        .child(mark)
                        .child(text),
                )
        };
        let mut rows: Vec<gpui::AnyElement> = self
            .shortcuts
            .iter()
            .map(|s| status_row(s.label.into(), s.keys.into(), &s.status).into_any_element())
            .collect();
        rows.push(
            status_row(
                "Tray icon".into(),
                "notification area".into(),
                &self.tray_status,
            )
            .into_any_element(),
        );
        let events: Vec<gpui::AnyElement> = if self.events.is_empty() {
            vec![
                div()
                    .text_color(color(MUTED))
                    .child("Press a shortcut, or click the tray icon.")
                    .into_any_element(),
            ]
        } else {
            self.events
                .iter()
                .map(|e| {
                    div()
                        .text_color(color(FOREGROUND))
                        .child(e.clone())
                        .into_any_element()
                })
                .collect()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(20.))
            .p(px(28.))
            .bg(color(BACKGROUND))
            .font_family("Segoe UI")
            .text_size(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(color(ACCENT))
                            .child("Tintcast"),
                    )
                    .child(
                        div()
                            .text_color(color(MUTED))
                            .child("Phase 0 shell: GPUI from Zed 37faef8, tray, global shortcuts"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .h(px(14.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(color(BORDER))
                    .children(ANSI.iter().map(|c| div().flex_1().h_full().bg(color(*c)))),
            )
            .child(div().flex().flex_col().children(rows))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(14.))
                    .rounded(px(6.))
                    .bg(color(SURFACE))
                    .border_1()
                    .border_color(color(BORDER))
                    .child(
                        div()
                            .text_color(color(MUTED))
                            .text_size(px(12.))
                            .child("EVENTS"),
                    )
                    .children(events),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(460.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Tintcast".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            let handle = window.window_handle();
            cx.new(|cx| Shell::new(handle, cx))
        });
        if let Err(error) = opened {
            eprintln!("could not open the window: {error:#}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
