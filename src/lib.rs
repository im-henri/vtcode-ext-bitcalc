pub mod app;
pub mod expr;
pub mod fmt;
pub mod line;
pub mod render;
#[cfg(test)]
mod tests;

#[cfg(target_arch = "wasm32")]
mod guest {
    use crate::app::{App, Event, Saved};
    use std::cell::RefCell;
    use vtcode_extension_api::{self as api, editor as host};

    const VIEW: &str = "bitcalc";
    const STATE_KEY: &str = "state";

    thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }

    fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> R {
        APP.with(|cell| {
            let mut slot = cell.borrow_mut();
            let app = slot.get_or_insert_with(|| {
                let mut app = App::new();
                if let Some(json) = api::load_setting(STATE_KEY)
                    && let Ok(saved) = serde_json::from_str::<Saved>(&json)
                {
                    app.load(saved);
                }
                app
            });
            f(app)
        })
    }

    fn flush(app: &mut App) {
        if let Some(text) = app.clipboard_out.take()
            && let Err(e) = host::set_clipboard(&text)
        {
            app.say(e, crate::app::Tone::Bad);
        }
        if std::mem::take(&mut app.save_needed)
            && let Ok(json) = serde_json::to_string(&app.save())
            && api::save_setting(STATE_KEY, &json) != 0
        {
            api::log("Bitcalc: could not save state");
        }
        let frame = app.render();
        if let Err(e) = host::set_frame(VIEW, &frame) {
            api::log(&e);
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn vtcode_activate() -> i32 {
        api::register_command("bitcalc.open", "Open Calculator");
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn vtcode_deactivate() -> i32 {
        APP.with(|cell| {
            if let Some(app) = cell.borrow().as_ref()
                && let Ok(json) = serde_json::to_string(&app.save())
            {
                api::save_setting(STATE_KEY, &json);
            }
        });
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn vtcode_run_command(index: i32) -> i32 {
        if index != 0 {
            return 1;
        }
        api::register_view(VIEW, "Bitcalc", "");
        api::register_view_key_handler(VIEW, "bitcalc_event");
        api::register_view_mouse_handler(VIEW, "bitcalc_event");
        api::register_view_scroll_handler(VIEW, "bitcalc_event");
        host::register_resize_handler(VIEW, "bitcalc_event");
        host::register_hover_handler(VIEW, "bitcalc_event");
        host::register_close_handler(VIEW, "bitcalc_event");
        host::enable_pointer_capture(VIEW, true);
        with_app(flush);
        api::open_view(VIEW);
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn bitcalc_event() -> i32 {
        let json = match api::current_view_event_limited(256 * 1024) {
            Ok(Some(json)) => json,
            Ok(None) => return 0,
            Err(e) => {
                api::set_status(&e);
                return 0;
            }
        };
        let Ok(mut event) = serde_json::from_str::<Event>(&json) else { return 0 };
        if event.code == "closed" {
            with_app(|app| {
                if let Ok(json) = serde_json::to_string(&app.save()) {
                    api::save_setting(STATE_KEY, &json);
                }
            });
            return 0;
        }
        if event.code == "paste" {
            if let Ok(text) = host::event_text_limited(64 * 1024) {
                event.text = Some(text);
            }
        } else if event.code == "command" && event.text.as_deref() == Some("paste") {
            match host::clipboard_limited(64 * 1024) {
                Ok(text) => {
                    event.code = "paste".into();
                    event.text = Some(text);
                }
                Err(e) => {
                    with_app(|app| {
                        app.say(e, crate::app::Tone::Bad);
                        flush(app);
                    });
                    return 1;
                }
            }
        }
        with_app(|app| {
            if app.event(&event) {
                flush(app);
            }
        });
        1
    }
}
