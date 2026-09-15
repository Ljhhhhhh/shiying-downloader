#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::oneshot,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    url: String,
    quality: String,
    title: String,
    status: String,
    progress: f64,
    detail: String,
    directory: String,
    file: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Store {
    directory: String,
    jobs: Vec<Job>,
}
struct Active {
    id: String,
    pid: u32,
    cancel: oneshot::Sender<()>,
}
struct AppState {
    store: Mutex<Store>,
    active: Mutex<Option<Active>>,
    config: PathBuf,
}

fn save(state: &AppState) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(&*state.store.lock().unwrap()).map_err(|e| e.to_string())?;
    let temp = state.config.with_extension("tmp");
    std::fs::write(&temp, bytes).map_err(|e| format!("无法保存下载记录：{e}"))?;
    // Windows rename cannot replace an existing file; retain the previous copy until the new bytes are ready.
    #[cfg(windows)]
    if state.config.exists() {
        std::fs::copy(&temp, &state.config).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(temp);
        return Ok(());
    }
    std::fs::rename(temp, &state.config).map_err(|e| e.to_string())
}
fn update(app: &tauri::AppHandle, id: &str, change: impl FnOnce(&mut Job)) {
    let state = app.state::<AppState>();
    let mut store = state.store.lock().unwrap();
    if let Some(job) = store.jobs.iter_mut().find(|j| j.id == id) {
        change(job);
        let _ = app.emit("download", job.clone());
    }
}
fn validate(url: &str, quality: &str) -> Result<String, String> {
    if url.len() > 8192 {
        return Err("链接过长，请重新复制视频链接。".into());
    }
    let parsed =
        url::Url::parse(url.trim()).map_err(|_| "链接格式不正确，请粘贴完整的视频链接。")?;
    if !["https", "http"].contains(&parsed.scheme())
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("仅支持不含账号密码的 http 或 https 链接。".into());
    }
    if !["best", "1080", "720", "audio"].contains(&quality) {
        return Err("请选择有效的画质。".into());
    }
    Ok(parsed.to_string())
}
fn args(url: &str, quality: &str, directory: &str, engines: &std::path::Path) -> Vec<String> {
    let runtime = engines.join(if cfg!(windows) { "deno.exe" } else { "deno" });
    let mut result: Vec<String> = [
        "--ignore-config",
        "--no-plugin-dirs",
        "--no-playlist",
        "--no-colors",
        "--newline",
        "--no-simulate",
        "--no-remote-components",
        "--no-js-runtimes",
        "--js-runtimes",
    ]
    .map(String::from)
    .into();
    result.extend([
        format!("deno:{}", runtime.display()),
        "--ffmpeg-location".into(),
        engines.to_string_lossy().into_owned(),
    ]);
    result.extend(
        [
            "--socket-timeout",
            "25",
            "--retries",
            "3",
            "--fragment-retries",
            "3",
            "--windows-filenames",
            "--no-overwrites",
            "--progress",
            "--progress-delta",
            "0.3",
            "--print",
            "before_dl:SY_META:%(.{title,id})j",
            "--progress-template",
            "download:SY_PROGRESS:%(progress)j",
            "--print",
            "after_move:SY_FILE:%(filepath)j",
            "-P",
            directory,
            "-o",
            if quality == "audio" {
                "%(title).150B [%(id)s] - audio.%(ext)s"
            } else {
                "%(title).150B [%(id)s].%(ext)s"
            },
        ]
        .map(String::from),
    );
    if quality == "audio" {
        result.extend(
            [
                "-f",
                "bestaudio/best",
                "-x",
                "--audio-format",
                "m4a",
                "--audio-quality",
                "0",
            ]
            .map(String::from),
        );
    } else {
        let cap = if quality == "best" {
            String::new()
        } else {
            format!("[height<={quality}]")
        };
        result.extend([
            "-f".into(),
            format!("bv*{cap}+ba/b{cap}"),
            "--merge-output-format".into(),
            "mkv".into(),
        ]);
    }
    result.extend(["--".into(), url.into()]);
    result
}
fn kill_tree(pid: u32, force: bool) {
    #[cfg(unix)]
    unsafe {
        libc::kill(
            -(pid as i32),
            if force { libc::SIGKILL } else { libc::SIGTERM },
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .status();
        let _ = force;
    }
}
#[tauri::command]
fn snapshot(state: State<AppState>) -> Store {
    state.store.lock().unwrap().clone()
}

#[tauri::command]
async fn choose_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let chosen = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择视频保存位置")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    chosen
        .map(|p| {
            p.into_path()
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| e.to_string())
        })
        .transpose()
}

#[tauri::command]
fn set_directory(directory: String, state: State<AppState>) -> Result<(), String> {
    let path = PathBuf::from(&directory);
    if !path.is_absolute() || !path.is_dir() {
        return Err("保存文件夹不存在，请重新选择。".into());
    }
    state.store.lock().unwrap().directory = directory;
    save(&state)
}

#[tauri::command]
async fn start_download(
    app: tauri::AppHandle,
    url: String,
    quality: String,
) -> Result<Job, String> {
    let url = validate(&url, &quality)?;
    let state = app.state::<AppState>();
    let mut active = state.active.lock().unwrap();
    if active.is_some() {
        return Err("已有任务正在下载，请完成或取消后再添加。".into());
    }
    let directory = state.store.lock().unwrap().directory.clone();
    std::fs::create_dir_all(&directory).map_err(|e| format!("无法使用保存文件夹：{e}"))?;
    let engines = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("engines");
    let executable = engines.join(if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    });
    let mut command = Command::new(executable);
    command
        .args(args(&url, &quality, &directory, &engines))
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动下载工具，请重新安装应用。详情：{e}"))?;
    let pid = child.id().ok_or("无法获取下载进程。")?;
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let job = Job {
        id: id.clone(),
        title: url::Url::parse(&url)
            .unwrap()
            .host_str()
            .unwrap_or("视频")
            .into(),
        url,
        quality,
        status: "preparing".into(),
        progress: 0.0,
        detail: "正在读取视频信息…".into(),
        directory,
        file: None,
    };
    state.store.lock().unwrap().jobs.insert(0, job.clone());
    if let Err(error) = save(&state) {
        kill_tree(pid, true);
        state.store.lock().unwrap().jobs.retain(|j| j.id != id);
        return Err(error);
    }
    let (tx, mut rx) = oneshot::channel();
    *active = Some(Active {
        id: id.clone(),
        pid,
        cancel: tx,
    });
    drop(active);
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let worker_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let error_reader = tauri::async_runtime::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut bytes = [0u8; 4096];
            let mut text = String::new();
            while let Ok(n) = reader.read(&mut bytes).await {
                if n == 0 {
                    break;
                }
                text.push_str(&String::from_utf8_lossy(&bytes[..n]));
                if text.len() > 16000 {
                    text = text
                        .chars()
                        .rev()
                        .take(8000)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                }
            }
            text
        });
        let event_app = worker_app.clone();
        let event_id = id.clone();
        let output_reader = tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                parse_line(&event_app, &event_id, &line);
            }
        });
        let (cancelled, result) = tokio::select! {
            result = child.wait() => (false, result),
            _ = &mut rx => {
                kill_tree(pid, false);
                let result = match tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await {
                    Ok(result) => result,
                    Err(_) => { kill_tree(pid, true); child.wait().await }
                };
                (true, result)
            }
        };
        let _ = output_reader.await;
        let error = error_reader.await.unwrap_or_default();
        let successful = result.map(|r| r.success()).unwrap_or(false);
        update(&worker_app, &id, |j| {
            j.status = if cancelled {
                "cancelled"
            } else if successful && j.file.is_some() {
                "done"
            } else {
                "error"
            }
            .into();
            j.detail = match j.status.as_str() {
                "done" => {
                    j.progress = 100.0;
                    "已保存到本地".into()
                }
                "cancelled" => "已取消，可重试继续下载".into(),
                _ => {
                    if error.trim().is_empty() {
                        "下载未完成，请检查链接和网络后重试。".into()
                    } else {
                        error.trim().into()
                    }
                }
            };
        });
        let state = worker_app.state::<AppState>();
        *state.active.lock().unwrap() = None;
        if let Err(error) = save(&state) {
            let _ = worker_app.emit("storage-error", error);
        }
        let _ = worker_app.emit("idle", ());
    });
    Ok(job)
}

fn parse_line(app: &tauri::AppHandle, id: &str, line: &str) {
    if let Some(text) = line.strip_prefix("SY_META:") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            if let Some(title) = value["title"].as_str() {
                update(app, id, |j| j.title = title.into());
            }
        }
    } else if let Some(text) = line.strip_prefix("SY_FILE:") {
        if let Ok(file) = serde_json::from_str::<String>(text) {
            update(app, id, |j| j.file = Some(file));
        }
    } else if let Some(text) = line.strip_prefix("SY_PROGRESS:") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            update(app, id, |j| {
                j.status = "downloading".into();
                let total = value["total_bytes"]
                    .as_f64()
                    .or_else(|| value["total_bytes_estimate"].as_f64())
                    .unwrap_or(0.0);
                let downloaded = value["downloaded_bytes"].as_f64().unwrap_or(0.0);
                j.progress = if total > 0.0 {
                    (downloaded / total * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };
                let speed = value["speed"]
                    .as_f64()
                    .map(|v| format!("{:.1} MB/s", v / 1_000_000.0))
                    .unwrap_or_else(|| "计算速度中".into());
                let eta = value["eta"]
                    .as_u64()
                    .map(|v| format!(" · 剩余 {}:{:02}", v / 60, v % 60))
                    .unwrap_or_default();
                j.detail = format!("{speed}{eta}");
                if value["status"] == "finished" {
                    j.status = "merging".into();
                    j.detail = "正在合并或整理文件…".into();
                }
            });
        }
    } else if line.starts_with("[Merger]") || line.starts_with("[ExtractAudio]") {
        update(app, id, |j| {
            j.status = "merging".into();
            j.detail = "正在合并或转换音频…".into();
        });
    }
}
#[tauri::command]
fn cancel_download(id: String, state: State<AppState>) -> Result<(), String> {
    let mut active = state.active.lock().unwrap();
    if active.as_ref().map(|a| a.id.as_str()) != Some(&id) {
        return Err("该任务已结束。".into());
    }
    // Keep the active slot occupied until the child and all postprocessors exit.
    let (dummy, _) = oneshot::channel();
    let sender = std::mem::replace(&mut active.as_mut().unwrap().cancel, dummy);
    let _ = sender.send(());
    Ok(())
}
#[tauri::command]
fn reveal_file(
    app: tauri::AppHandle,
    id: Option<String>,
    state: State<AppState>,
) -> Result<(), String> {
    let store = state.store.lock().unwrap();
    if let Some(id) = id {
        let file = store
            .jobs
            .iter()
            .find(|j| j.id == id)
            .and_then(|j| j.file.as_ref())
            .ok_or("文件尚未下载完成。")?;
        if !PathBuf::from(file).is_file() {
            return Err("文件已移动或删除，请查看保存文件夹。".into());
        }
        app.opener()
            .reveal_item_in_dir(file)
            .map_err(|e| e.to_string())
    } else {
        std::fs::create_dir_all(&store.directory)
            .map_err(|e| format!("无法打开保存文件夹，请重新选择位置：{e}"))?;
        app.opener()
            .open_path(&store.directory, None::<&str>)
            .map_err(|e| e.to_string())
    }
}
#[tauri::command]
fn clear_history(state: State<AppState>) -> Result<(), String> {
    if state.active.lock().unwrap().is_some() {
        return Err("请等待当前下载结束后清空记录。".into());
    }
    state.store.lock().unwrap().jobs.clear();
    save(&state)
}
fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&dir)?;
            let config = dir.join("downloads.json");
            let mut store: Store = match std::fs::read(&config) {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(value) => value,
                    Err(_) => {
                        std::fs::copy(
                            &config,
                            dir.join(format!(
                                "downloads-backup-{}.json",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)?
                                    .as_secs()
                            )),
                        )?;
                        Store::default()
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
                Err(e) => return Err(e.into()),
            };
            if store.directory.is_empty() {
                store.directory = app
                    .path()
                    .download_dir()?
                    .join("拾影")
                    .to_string_lossy()
                    .into_owned();
            }
            for job in &mut store.jobs {
                if ["preparing", "downloading", "merging"].contains(&job.status.as_str()) {
                    job.status = "cancelled".into();
                    job.detail = "上次下载被中断，可重试继续下载".into();
                }
            }
            app.manage(AppState {
                store: Mutex::new(store),
                active: Mutex::new(None),
                config,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            choose_directory,
            set_directory,
            start_download,
            cancel_download,
            reveal_file,
            clear_history
        ])
        .build(tauri::generate_context!())
        .expect("无法启动拾影");
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(active) = app.state::<AppState>().active.lock().unwrap().as_ref() {
                kill_tree(active.pid, true);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_inputs_and_builds_safe_arguments() {
        for bad in [
            "file:///etc/passwd",
            "--exec=bad",
            "https://user:secret@example.com/video",
            "not a url",
        ] {
            assert!(validate(bad, "best").is_err());
        }
        assert!(validate("https://example.com/a", "evil").is_err());
        let url = validate(" https://example.com/video?a=1&b=2 ", "1080").unwrap();
        let arguments = args(
            &url,
            "1080",
            "/tmp/with spaces",
            std::path::Path::new("/engines"),
        );
        assert_eq!(&arguments[arguments.len() - 2..], &["--", &url]);
        assert!(arguments.contains(&"bv*[height<=1080]+ba/b[height<=1080]".into()));
        assert!(arguments.contains(&"/tmp/with spaces".into()));
        assert!(arguments.contains(&"--ignore-config".into()));
        let audio = args(&url, "audio", "/tmp", std::path::Path::new("/engines"));
        assert!(audio.windows(2).any(|v| v == ["--audio-format", "m4a"]));
    }
}
