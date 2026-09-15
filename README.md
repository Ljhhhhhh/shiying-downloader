# 拾影

基于 Tauri 2 和 yt-dlp 的中文视频下载器。安装包内置 yt-dlp、FFmpeg、FFprobe、Deno；Windows 安装包附带 WebView2 离线安装程序。

支持 macOS 12 及以上（Apple 芯片、Intel 分别打包），Windows 10/11 x64。

## 使用

1. macOS：打开 DMG，将「拾影」拖入 Applications；Windows：运行 `setup.exe` 并完成安装。
2. 粘贴完整视频链接，选择最佳画质、最高 1080p、最高 720p 或 M4A 音频。
3. 点击「开始下载」。默认保存至系统下载文件夹内的「拾影」，也可以更改位置。

一次下载一个视频，不展开播放列表。分离的音视频自动合并为 MKV；已有完整音视频文件保留源格式。取消后可以重试，服务器支持续传时会利用未完成文件。清空记录不会删除已下载文件。

网站可用性取决于网络、站点规则和 yt-dlp 支持情况。本版本不含账号登录、Cookie 导入、付费或 DRM 内容解密。需要登录或受访问限制的视频可能无法下载。关闭应用会中断当前任务，下次打开可重试。

当前构建没有开发者发布签名或 Apple 公证，首次安装或打开可能出现系统安全提示。正式公开分发需使用自己的 Apple Developer ID / Windows 代码签名证书。

## 开发与构建

构建环境需要 Node.js、Rust 和对应平台的 Tauri 原生工具链。普通使用者不需要安装这些工具。

```sh
npm ci
# Apple 芯片 Mac
npm run prepare:engines -- mac-arm64
npm run dev
npm test
npm run build

# Intel Mac（可在 Apple 芯片 Mac 上交叉构建）
npm run prepare:engines -- mac-x64
rustup target add x86_64-apple-darwin
npm run tauri build -- --target x86_64-apple-darwin --config src-tauri/tauri.intel.conf.json

# Windows 原生构建
npm run prepare:engines -- win-x64
npm run tauri build -- --target x86_64-pc-windows-msvc --config src-tauri/tauri.win.conf.json
```

macOS 引擎准备脚本会从 FFmpeg 7.1.1 源码构建 LGPL 版本，需要 Xcode 命令行工具和 make。Windows 使用 FFmpeg 6.1.1 GPL 版本。引擎的版本和 SHA-256 摘要随安装包保存在 `engines/versions.json`。

在 macOS 交叉构建 Windows NSIS 安装包：

```sh
brew install nsis llvm
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
npm run prepare:engines -- win-x64
PATH="/opt/homebrew/opt/llvm/bin:$PATH" npm run tauri build -- --runner cargo-xwin --target x86_64-pc-windows-msvc --config src-tauri/tauri.win.conf.json
```

产物位于 `src-tauri/target/<target>/release/bundle/`；本机默认架构构建位于 `src-tauri/target/release/bundle/`。Windows 构建会下载微软 WebView2 离线安装器。

## 检查

`npm test` 验证链接与画质校验、参数隔离和音频选项。运行 `python3 tests/fixture-server.py` 可启动本地测试视频服务：在桌面界面下载 `http://127.0.0.1:18767/sample.mp4`，使用 `/slow.mp4` 测试取消，再测试音频提取、重试、重启后的记录和显示文件。

界面使用系统 WebView，开启 CSP。下载工具通过参数数组调用，禁用用户 yt-dlp 配置、第三方插件和远程组件加载；前端没有任意命令执行接口。

## 开源许可

应用源码按 GPL-3.0-or-later 发布，见 `assets/LICENSE.txt`。随包携带的工具分别保留各自许可与第三方声明。

- yt-dlp：<https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19>
- macOS FFmpeg 源码：<https://ffmpeg.org/releases/ffmpeg-7.1.1.tar.xz>；构建命令见 `scripts/build-ffmpeg-mac.sh`。
- Windows FFmpeg 构建与源代码信息：<https://github.com/eugeneware/ffmpeg-static/releases/tag/b6.1.1>，以随包 `FFmpeg-README.txt` 为准。
- Deno：<https://github.com/denoland/deno/releases/tag/v2.9.6>
- Tauri：<https://github.com/tauri-apps/tauri>

依赖固定在 `package-lock.json` 与 `src-tauri/Cargo.lock`。更新下载站点支持时，调整 `scripts/engines.mjs` 的 yt-dlp 版本并重新构建安装包。
