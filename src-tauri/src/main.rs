#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};
use tauri::{Emitter, Manager, State};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::oneshot,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    url: String,
    #[serde(default)]
    source: String,
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
    pending: Mutex<Option<Import>>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Import {
    video_id: String,
    page: String,
    title: String,
    media: String,
    audio: String,
    duration: f64,
    width: u32,
    height: u32,
}
const REFERER: &str = "https://www.douyin.com/";

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
    if !["best", "1080", "720", "audio"].contains(&quality) {
        return Err("请选择有效的画质。".into());
    }
    let parse_link = |candidate: &str| {
        let parsed = url::Url::parse(candidate).ok()?;
        (["https", "http"].contains(&parsed.scheme())
            && parsed.host_str().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none())
        .then(|| parsed.to_string())
    };
    if let Some(link) = parse_link(url.trim()) {
        return Ok(link);
    }
    for (index, _) in url.match_indices("http") {
        let candidate = &url[index..];
        if !candidate.starts_with("https://") && !candidate.starts_with("http://") {
            continue;
        }
        let candidate = candidate
            .split(|c: char| c.is_whitespace() || "<>[](){}\"'，。！？、；;".contains(c))
            .next()
            .unwrap_or("");
        if let Some(link) = parse_link(candidate) {
            return Ok(link);
        }
    }
    Err("未找到有效的视频链接，请粘贴完整链接或分享内容。".into())
}
fn args_for(
    url: &str,
    quality: &str,
    directory: &str,
    engines: &std::path::Path,
    referer: Option<&str>,
) -> Vec<String> {
    args_named(url, quality, directory, engines, referer, None)
}
fn safe_stem(title: &str, id: &str, audio: bool) -> String {
    let mut stem: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "- _".contains(c) {
                c
            } else {
                ' '
            }
        })
        .collect();
    stem = stem.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem: String = stem.chars().take(50).collect();
    let stem = if stem.is_empty() {
        "douyin"
    } else {
        stem.trim()
    };
    format!(
        "{stem} [{id}]{}.%(ext)s",
        if audio { " - audio" } else { " - video" }
    )
}
fn args_named(
    url: &str,
    quality: &str,
    directory: &str,
    engines: &std::path::Path,
    referer: Option<&str>,
    name: Option<(&str, &str)>,
) -> Vec<String> {
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
    let output = if let Some((title, id)) = name {
        safe_stem(title, id, quality == "audio")
    } else if quality == "audio" {
        "%(title).150B [%(id)s] - audio.%(ext)s".into()
    } else {
        "%(title).150B [%(id)s].%(ext)s".into()
    };
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
            &output,
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
            (if name.is_some() { "mp4" } else { "mkv" }).into(),
        ]);
    }
    if let Some(referer) = referer {
        result.extend(["--referer".into(), referer.into()]);
    }
    result.extend(["--".into(), url.into()]);
    result
}
fn vod_url(input: &str, kind: &str) -> Result<String, String> {
    let media = validate(input, "best")?;
    let parsed = url::Url::parse(&media).unwrap();
    let host = parsed.host_str().unwrap_or("");
    if parsed.scheme() != "https"
        || parsed.port().is_some()
        || !(host == "douyinvod.com" || host.ends_with(".douyinvod.com"))
        || !parsed.path().contains(&format!("/media-{kind}-"))
    {
        return Err("画面或声音地址无效，请回到抖音播放后重新发送。".into());
    }
    Ok(media)
}
fn import_from(link: &url::Url) -> Result<Import, String> {
    if link.scheme() != "shiying" || link.host_str() != Some("download") {
        return Err("无法识别的拾影链接。".into());
    }
    if link.as_str().len() > 16 * 1024 {
        return Err("发送内容过大。".into());
    }
    let q = |name: &str| {
        link.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("缺少{name}"))
    };
    if q("v")? != "2" {
        return Err("扩展版本不受支持，请在 Chrome 扩展程序页面重新加载拾影。".into());
    }
    let video_id = q("id")?;
    if video_id.len() > 32 || !video_id.chars().all(|c| c.is_ascii_digit()) {
        return Err("视频编号无效。".into());
    }
    let page = q("page")?;
    let page_url = url::Url::parse(&page).map_err(|_| "原视频页面无效。")?;
    if page_url.scheme() != "https"
        || !matches!(page_url.host_str(), Some("www.douyin.com" | "v.douyin.com"))
        || page.len() > 300
    {
        return Err("原视频页面无效。".into());
    }
    let title = q("title")?;
    if title.chars().count() > 120 {
        return Err("标题过长。".into());
    }
    let media = vod_url(&q("media")?, "video")?;
    let audio = vod_url(&q("audio")?, "audio")?;
    let media_url = url::Url::parse(&media).unwrap();
    let audio_url = url::Url::parse(&audio).unwrap();
    let marker = |url: &url::Url| {
        url.query_pairs()
            .find(|(key, _)| key == "l")
            .map(|(_, value)| value.into_owned())
    };
    if marker(&media_url).is_none() || marker(&media_url) != marker(&audio_url) {
        return Err("画面和声音不属于同一次播放，请重新发送。".into());
    }
    let duration: f64 = q("duration")?.parse().map_err(|_| "时长无效。")?;
    let width: u32 = q("width")?.parse().map_err(|_| "分辨率无效。")?;
    let height: u32 = q("height")?.parse().map_err(|_| "分辨率无效。")?;
    if !(0.0..=86_400.0).contains(&duration) || width > 7680 || height > 4320 {
        return Err("视频信息不合理。".into());
    }
    Ok(Import {
        video_id,
        page,
        title,
        media,
        audio,
        duration,
        width,
        height,
    })
}
fn offer(app: &tauri::AppHandle, link: &url::Url) {
    match import_from(link) {
        Ok(import) => {
            let state = app.state::<AppState>();
            let mut slot = state.pending.lock().unwrap();
            if slot.is_some() {
                let _ = app.emit("import-busy", "已有待确认的视频，请先下载或取消。");
                return;
            }
            *slot = Some(import.clone());
            let _ = app.emit("import", import);
        }
        Err(error) => {
            let _ = app.emit("import-error", error);
        }
    }
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
fn pending_import(state: State<AppState>) -> Option<Import> {
    state.pending.lock().unwrap().clone()
}
#[tauri::command]
fn dismiss_import(state: State<AppState>) {
    *state.pending.lock().unwrap() = None;
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
    let state = app.state::<AppState>();
    let direct = state
        .pending
        .lock()
        .unwrap()
        .clone()
        .filter(|item| item.media == url);
    let referer = direct.as_ref().map(|_| REFERER);
    let name = direct
        .as_ref()
        .map(|item| (item.title.clone(), item.video_id.clone()));
    let url = validate(&url, &quality)?;
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
    let mut args = args_named(
        &url,
        &quality,
        &directory,
        &engines,
        referer,
        name.as_ref()
            .map(|(title, id)| (title.as_str(), id.as_str())),
    );
    let info = direct.as_ref().map(|item| {
        serde_json::to_vec(&serde_json::json!({
            "id": item.video_id,
            "title": item.title,
            "webpage_url": item.page,
            "extractor": "generic",
            "formats": [
                { "format_id": "video", "url": item.media, "vcodec": "hvc1", "acodec": "none", "ext": "mp4", "width": item.width, "height": item.height },
                { "format_id": "audio", "url": item.audio, "vcodec": "none", "acodec": "mp4a", "ext": "m4a" }
            ]
        })).map_err(|e| e.to_string())
    }).transpose()?;
    if info.is_some() {
        args.truncate(args.len() - 2);
        args.extend(["--load-info-json".into(), "-".into()]);
    }
    command
        .args(args)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(if info.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
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
        title: direct
            .as_ref()
            .map(|item| item.title.clone())
            .unwrap_or_else(|| {
                url::Url::parse(&url)
                    .unwrap()
                    .host_str()
                    .unwrap_or("视频")
                    .into()
            }),
        url: direct.as_ref().map(|item| item.page.clone()).unwrap_or(url),
        source: if direct.is_some() {
            "douyin".into()
        } else {
            String::new()
        },
        quality,
        status: "preparing".into(),
        progress: 0.0,
        detail: "正在读取视频信息…".into(),
        directory,
        file: None,
    };
    state.store.lock().unwrap().jobs.insert(0, job.clone());
    if direct.is_some() {
        *state.pending.lock().unwrap() = None;
    }
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
        let input_error = if let Some(info) = info {
            child.stdin.take().unwrap().write_all(&info).await.err()
        } else {
            None
        };
        if input_error.is_some() {
            kill_tree(pid, true);
        }
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
        let (cancelled, result) = if input_error.is_some() {
            (false, child.wait().await)
        } else {
            tokio::select! {
                result = child.wait() => (false, result),
                _ = &mut rx => {
                    kill_tree(pid, false);
                    let result = match tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await {
                        Ok(result) => result,
                        Err(_) => { kill_tree(pid, true); child.wait().await }
                    };
                    (true, result)
                }
            }
        };
        let _ = output_reader.await;
        let mut error = error_reader.await.unwrap_or_default();
        if let Some(input_error) = input_error {
            error = format!("无法发送视频信息给下载工具：{input_error}");
        }
        if let Some(import) = &direct {
            error = error
                .replace(&import.media, "[画面地址]")
                .replace(&import.audio, "[声音地址]");
        }
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
        .plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {}))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
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
                pending: Mutex::new(None),
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                if let Some(link) = urls.into_iter().next() {
                    offer(app.handle(), &link);
                }
            }
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                if let Some(link) = event.urls().into_iter().next() {
                    offer(&handle, &link);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            pending_import,
            dismiss_import,
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
        let arguments = args_for(
            &url,
            "1080",
            "/tmp/with spaces",
            std::path::Path::new("/engines"),
            None,
        );
        assert_eq!(&arguments[arguments.len() - 2..], &["--", &url]);
        assert!(arguments.contains(&"bv*[height<=1080]+ba/b[height<=1080]".into()));
        assert!(arguments.contains(&"/tmp/with spaces".into()));
        assert!(arguments.contains(&"--ignore-config".into()));
        let audio = args_for(
            &url,
            "audio",
            "/tmp",
            std::path::Path::new("/engines"),
            None,
        );
        assert!(audio.windows(2).any(|v| v == ["--audio-format", "m4a"]));
    }
}
