// SPDX-License-Identifier: Apache-2.0
//! Ghira iOS app: a Tauri shell over the shared `ghi-app` commands, the
//! mobile-owned commands in [`cmd`] (recording, lifecycle, models, privacy,
//! inbox, onboarding, voice) and the Swift side over the C ABI in
//! [`platform`].
//!
//! Commands and events are typed with tauri-specta; regenerate the TypeScript
//! bindings with `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-mobile`.
//!
//! The spike session (`session`, `engine`, `gate`, `backlog`) stays until
//! slice 16-D rewrites it onto `ghi-core`; nothing registers its commands any
//! more, so only the Live Activity intents and the self-test reach it.

#[allow(dead_code)] // replaced by 16-D
mod backlog;
pub mod cmd;
#[allow(dead_code)] // replaced by 16-D
mod engine;
#[allow(dead_code)] // replaced by 16-D
mod gate;
mod platform;
#[allow(dead_code)] // replaced by 16-D
mod session;
#[cfg(feature = "test-hooks")]
mod spikes;

use std::sync::Arc;

use tauri::Manager;

// The same navigation guard as the desktop app (RT-6).
#[path = "../../../desktop/src-tauri/src/navigation.rs"]
mod navigation;

/// Writes the TypeScript bindings for all commands and events to `path`.
pub fn export_bindings(path: &str) {
    cmd::builder()
        .export(
            specta_typescript::Typescript::default()
                .header("// SPDX-License-Identifier: Apache-2.0\n"),
            path,
        )
        .expect("failed to export TypeScript bindings");
}

/// iOS: ggml's Metal residency sets keep the weights wired by a background
/// heartbeat after the last compute; while the phone sits locked for an hour
/// that is jetsam bait, so they are off (read when the first model loads).
fn configure_ggml() {
    // SAFETY: runs first thing in `run`, before any thread of ours exists and
    // before anything reads the environment.
    #[cfg(target_os = "ios")]
    unsafe {
        std::env::set_var("GGML_METAL_NO_RESIDENCY", "1")
    };
}

/// iOS (spike): stderr goes to `Documents/logs/<unix time>.log`, so ggml's
/// Metal errors (a refused background submission is only logged) can be
/// checked after a lock test.
#[cfg(target_os = "ios")]
fn capture_stderr(data: &std::path::Path) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let dir = data.join("logs");
    std::fs::create_dir_all(&dir)?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let file = std::fs::File::create(dir.join(format!("{secs}.log")))?;
    // SAFETY: both descriptors are valid; fd 2 now refers to the log file,
    // which stays open for the life of the process (leaked below).
    if unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    std::mem::forget(file);
    Ok(())
}

/// Spike self-test: launched with `GHI_SELFTEST=<file in Documents>` (see
/// apps/mobile/scripts/selftest-ios.sh), runs that 16 kHz WAV through the
/// engine as fast as possible and writes `Documents/selftest-<unix>.json`.
fn selftest_on_launch(data: &std::path::Path) {
    let Some(name) = std::env::var_os("GHI_SELFTEST") else {
        return;
    };
    let data = data.to_path_buf();
    std::thread::spawn(move || {
        let file = name.to_string_lossy().into_owned();
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let result = match std::fs::read(data.join(&file)) {
            Err(e) => serde_json::json!({"file": file, "error": e.to_string()}),
            Ok(bytes) => match engine::read_wav_16k(&bytes) {
                Err(e) => serde_json::json!({"file": file, "error": e}),
                #[cfg(feature = "nemo")]
                Ok(pcm) => serde_json::to_value(engine::selftest(&data.join("models"), file, &pcm))
                    .unwrap_or_default(),
                #[cfg(not(feature = "nemo"))]
                Ok(_) => {
                    serde_json::json!({"file": file, "error": "no speech engine in this build"})
                }
            },
        };
        let path = data.join(format!("selftest-{secs}.json"));
        if let Err(e) = std::fs::write(&path, result.to_string()) {
            eprintln!("ghira: self-test result: {e}");
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    configure_ggml();
    #[cfg(feature = "test-hooks")]
    eprintln!("ghira: TEST HOOKS ENABLED (simulator build)");
    let builder = cmd::builder();
    let tauri_builder = tauri::Builder::default();
    #[cfg(feature = "test-hooks")]
    let tauri_builder =
        tauri_builder.register_uri_scheme_protocol("ghi-audio", spikes::audio_scheme);
    tauri_builder
        .plugin(navigation::guard())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            cmd::events::install(app.handle());
            // The store-facing state the shared commands take. Nothing is
            // opened here: 16-G builds the core's data dir and starts it.
            app.manage(Arc::new(ghi_app::core::Core::new(app.handle(), |_| {})?));
            app.manage(Arc::new(ghi_app::audio_protocol::AudioTokens::default()));
            app.manage(Arc::new(ghi_app::cloud_cmd::CloudPlans::default()));
            // iOS: Documents (visible in the Files app, where the owner pulls
            // recordings and metrics); elsewhere the app data dir.
            let data = if cfg!(target_os = "ios") {
                app.path().document_dir()?
            } else {
                app.path().app_data_dir()?
            };
            std::fs::create_dir_all(data.join("models"))?;
            // Recordings are unencrypted in the spike: never in device backups.
            if let Err(e) = platform::exclude_from_backup(&data) {
                eprintln!("ghira: {e}");
            }
            #[cfg(target_os = "ios")]
            if let Err(e) = capture_stderr(&data) {
                eprintln!("ghira: could not capture stderr: {e}");
            }
            selftest_on_launch(&data);
            platform::init();
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            );
            #[cfg(feature = "test-hooks")]
            let window = if std::env::var("GHI_SPIKE").as_deref() == Ok("audio") {
                window.initialization_script(spikes::AUDIO_SCRIPT)
            } else {
                window
            };
            window
                .title("Ghira")
                .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Ghira app");
}

#[cfg(test)]
mod tests {
    /// Keeps the committed bindings in sync; CI fails if they drift.
    /// Regenerate with `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-mobile`.
    #[test]
    fn bindings_are_up_to_date() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");
        let tmp =
            std::env::temp_dir().join(format!("ghi-mobile-bindings-{}.ts", std::process::id()));
        super::export_bindings(tmp.to_str().unwrap());
        let fresh = std::fs::read_to_string(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);
        if std::env::var_os("GHI_UPDATE_BINDINGS").is_some() {
            std::fs::write(path, &fresh).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(
            committed, fresh,
            "src/bindings.ts is stale: run `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-mobile`"
        );
    }

    /// `build.rs`, `capabilities/default.json` and the registered commands
    /// (as written in the bindings) name the same set.
    #[test]
    fn commands_are_granted() {
        let dir = env!("CARGO_MANIFEST_DIR");
        let read = |p: &str| std::fs::read_to_string(format!("{dir}/{p}")).unwrap();
        let mut built: Vec<String> = read("build.rs")
            .lines()
            .filter_map(|l| {
                let name = l.trim().strip_prefix('"')?.strip_suffix("\",")?;
                Some(name.to_owned())
            })
            .collect();
        let capability: serde_json::Value =
            serde_json::from_str(&read("capabilities/default.json")).unwrap();
        let permissions: Vec<&str> = capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        // Exactly core:default plus one allow-* per command: nothing else is granted.
        let others: Vec<&str> = permissions
            .iter()
            .copied()
            .filter(|p| !p.starts_with("allow-"))
            .collect();
        assert_eq!(others, ["core:default"]);
        let mut granted: Vec<String> = permissions
            .iter()
            .filter_map(|p| Some(p.strip_prefix("allow-")?.replace('-', "_")))
            .collect();
        let mut bound: Vec<String> = read("../src/bindings.ts")
            .lines()
            .filter_map(|l| {
                let rest = l.strip_prefix('\t')?.split("__TAURI_INVOKE").nth(1)?;
                Some(rest.split('"').nth(1)?.to_owned())
            })
            .collect();
        for v in [&mut built, &mut granted, &mut bound] {
            v.sort();
        }
        let diff = |a: &[String], b: &[String]| -> Vec<String> {
            a.iter().filter(|x| !b.contains(x)).cloned().collect()
        };
        assert!(
            built == bound,
            "build.rs vs bindings: only in build.rs {:?}, only in bindings {:?}",
            diff(&built, &bound),
            diff(&bound, &built)
        );
        assert!(
            granted == bound,
            "capabilities vs bindings: only granted {:?}, only in bindings {:?}",
            diff(&granted, &bound),
            diff(&bound, &granted)
        );
    }
}
