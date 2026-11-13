use std::sync::{Arc, Mutex};

pub fn start(status: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<String> {
            let update = self_update::backends::github::Update::configure()
                .repo_owner("roc198419")
                .repo_name("roc-request-bin")
                .identifier("roc-request-bin-")
                .bin_name("roc-request-bin")
                .bin_path_in_archive(if cfg!(windows) {
                    "roc-request-bin.exe"
                } else {
                    "roc-request-bin"
                })
                .current_version(env!("CARGO_PKG_VERSION"))
                .no_confirm(true)
                .show_download_progress(false)
                .build()?
                .update()?;
            Ok(if update.updated() {
                format!("已更新至 {}，重启后生效", update.version())
            } else {
                "已是最新版本".into()
            })
        })();
        *status.lock().unwrap() = match result {
            Ok(s) => s,
            Err(e) => format!("更新检查失败：{e}"),
        };
    });
}
