#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod tray;
mod ui;
mod update;

use gpui_kit::assets::Assets;
use gpui_kit::*;
use roc_request_bin::store::Store;

gpui_kit::assets::icon_assets!(EditAssets, [Pencil]);

struct AppAssets;
impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match EditAssets.load(path)? {
            Some(icon) => Ok(Some(icon)),
            None => Assets.load(path),
        }
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = Assets.list(path)?;
        assets.extend(EditAssets.list(path)?);
        Ok(assets)
    }
}

fn main() {
    let store = match Store::user() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("Cannot open ~/.roc-request-bin/data.db: {error:#}");
            return;
        }
    };
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
                window_min_size: Some(size(px(960.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Roc Request Bin".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| ui::Inbox::new(store, window, cx))
            })
            .expect("Open application window");
            cx.activate(true);
        });
}
