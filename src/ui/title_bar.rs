use dioxus::desktop::use_window;
use dioxus::prelude::*;

/// Custom title bar for the frameless window: drag region + window controls.
#[component]
pub fn TitleBar() -> Element {
    let desktop = use_window();
    let mut is_maximized = use_signal(|| desktop.window.is_maximized());

    let drag_desktop = desktop.clone();
    let minimize_desktop = desktop.clone();
    let maximize_desktop = desktop.clone();
    let close_desktop = desktop.clone();

    rsx! {
        div {
            class: "flex items-center h-8 bg-black/60 select-none flex-shrink-0 border-b border-white/5",

            // Left: draggable title area
            div {
                class: "flex items-center gap-2 px-3 flex-1",
                onmousedown: move |_| drag_desktop.drag(),
                div {
                    class: "flex items-center justify-center w-4 h-4",
                    span {
                        class: "text-[10px] font-bold text-indigo-400",
                        style: "font-family: 'JetBrains Mono', monospace;",
                        "K"
                    }
                }
                span {
                    class: "text-xs text-slate-400",
                    style: "font-family: 'JetBrains Mono', monospace;",
                    "Miko-Kagura"
                }
            }

            // Right: window control buttons (not draggable)
            div {
                class: "flex items-center -mr-2",

                button {
                    class: "w-11 h-8 flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors cursor-pointer",
                    onclick: move |_| {
                        minimize_desktop.window.set_minimized(true);
                    },
                    svg {
                        class: "w-3 h-3",
                        view_box: "0 0 12 12",
                        stroke: "currentColor",
                        "stroke-width": "1.5",
                        fill: "none",
                        line { x1: "1", y1: "6", x2: "11", y2: "6" }
                    }
                }

                button {
                    class: "w-11 h-8 flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors cursor-pointer",
                    onclick: move |_| {
                        maximize_desktop.toggle_maximized();
                        is_maximized.set(maximize_desktop.window.is_maximized());
                    },
                    if *is_maximized.read() {
                        svg {
                            class: "w-3 h-3",
                            view_box: "0 0 12 12",
                            stroke: "currentColor",
                            "stroke-width": "1.2",
                            fill: "none",
                            rect { x: "2.5", y: "0.5", width: "9", height: "9", rx: "1" }
                            path { d: "M1.5 3.5V10.5C1.5 11.05 1.95 11.5 2.5 11.5H9.5", stroke: "currentColor", fill: "none" }
                        }
                    } else {
                        svg {
                            class: "w-3 h-3",
                            view_box: "0 0 12 12",
                            stroke: "currentColor",
                            "stroke-width": "1.2",
                            fill: "none",
                            rect { x: "1.5", y: "1.5", width: "9", height: "9", rx: "1" }
                        }
                    }
                }

                button {
                    class: "w-11 h-8 flex items-center justify-center text-slate-400 hover:text-white hover:bg-red-500 transition-colors cursor-pointer rounded-tr-md",
                    onclick: move |_| {
                        // Settings are saved on every change, but flush once
                        // more here so a value still mid-edit isn't lost.
                        crate::settings::persist(crate::app_state().as_ref());
                        close_desktop.close();
                    },
                    svg {
                        class: "w-3 h-3",
                        view_box: "0 0 12 12",
                        stroke: "currentColor",
                        "stroke-width": "1.5",
                        fill: "none",
                        line { x1: "1", y1: "1", x2: "11", y2: "11" }
                        line { x1: "11", y1: "1", x2: "1", y2: "11" }
                    }
                }
            }
        }
    }
}
