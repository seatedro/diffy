#[cfg(feature = "hot-reload")]
mod inner {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;

    use serde::Deserialize;
    use winit::event_loop::EventLoopProxy;

    const DEVSERVER_IP_ENV: &str = "DIOXUS_DEVSERVER_IP";
    const DEVSERVER_PORT_ENV: &str = "DIOXUS_DEVSERVER_PORT";
    const BUILD_ID_ENV: &str = "DIOXUS_BUILD_ID";

    #[derive(Deserialize)]
    enum DevserverMsg {
        HotReload(HotReloadMsg),
        #[serde(other)]
        Other,
    }

    #[derive(Deserialize)]
    struct HotReloadMsg {
        jump_table: Option<subsecond::JumpTable>,
        for_pid: Option<u32>,
    }

    fn devserver_endpoint() -> Option<String> {
        let ip = std::env::var(DEVSERVER_IP_ENV).ok()?;
        let port = std::env::var(DEVSERVER_PORT_ENV).ok()?;
        Some(format!("ws://{ip}:{port}/_dioxus"))
    }

    fn build_id() -> u64 {
        std::env::var(BUILD_ID_ENV)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn connect(proxy: EventLoopProxy<()>, patch_pending: Arc<AtomicBool>) {
        let Some(endpoint) = devserver_endpoint() else {
            return;
        };

        thread::spawn(move || {
            let uri = format!(
                "{endpoint}?aslr_reference={}&build_id={}&pid={}",
                subsecond::aslr_reference(),
                build_id(),
                std::process::id(),
            );

            let Ok((mut ws, _)) = tungstenite::connect(&uri) else {
                tracing::warn!("hot reload: failed to connect to dev server at {endpoint}");
                return;
            };

            tracing::info!("hot reload: connected to dev server at {endpoint}");

            while let Ok(msg) = ws.read() {
                let tungstenite::Message::Text(text) = msg else {
                    continue;
                };

                let Ok(DevserverMsg::HotReload(hot_reload)) = serde_json::from_str(&text) else {
                    continue;
                };

                let Some(jump_table) = hot_reload.jump_table else {
                    continue;
                };

                if hot_reload.for_pid != Some(std::process::id()) {
                    continue;
                }

                match unsafe { subsecond::apply_patch(jump_table) } {
                    Ok(()) => {
                        tracing::info!("hot reload: patch applied");
                        patch_pending.store(true, Ordering::Release);
                        let _ = proxy.send_event(());
                    }
                    Err(err) => {
                        tracing::error!("hot reload: patch failed: {err}");
                    }
                }
            }

            tracing::info!("hot reload: dev server disconnected");
        });
    }
}

#[cfg(feature = "hot-reload")]
pub use inner::connect;
