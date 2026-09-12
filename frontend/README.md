# HuiTrace 桌面端开发

本目录包含 Next.js 界面与 Tauri / Rust 核心。产品功能和使用方法见 [项目首页](../README.md)。当前版本为 **1.1.0**，版本号分别记录在 `package.json`、`src-tauri/Cargo.toml` 和 `src-tauri/tauri.conf.json` 中。

## 工具与依赖

| 工具 | 说明 |
| --- | --- |
| Node.js | CI 使用 20.19.4 |
| pnpm | `package.json` 固定为 10.33.0 |
| Rust | stable 工具链；桌面端及辅助程序在同一 Cargo workspace 中 |
| Python 3 | 下载并校验 sherpa-onnx 原生依赖，不是应用运行时后端 |
| C/C++、CMake | 用于构建语音和本地 AI 原生组件 |

Windows 需要 Visual Studio C++ Build Tools、Windows SDK 和 WebView2；macOS 需要 Xcode Command Line Tools；Linux 需要 WebKitGTK、GTK、音频等 Tauri 系统依赖。原生构建还可能需要 LLVM / libclang。平台依赖细节可参考 [构建资料](../docs/BUILDING.md) 和 [CI 构建流程](../.github/workflows/build.yml)，旧资料中的品牌名不影响代码路径。

## 安装 JavaScript 依赖

从仓库根目录进入：

```bash
cd frontend
pnpm install --frozen-lockfile
```

以下辅助程序准备命令从 **仓库根目录** 执行。首次构建需要联网获取依赖，完成后模型仍需在应用中单独准备。

## 准备辅助程序

Tauri 配置声明了 `llama-helper`、`diarize-helper` 和 `ffmpeg` 三个外部程序。前两个需要构建并放入 `frontend/src-tauri/binaries/`，文件名必须包含 Rust target triple。FFmpeg 由应用构建脚本处理下载与校验。

### Windows / PowerShell

在安装好 Python 的终端中执行；如果系统使用 `py -3`，将下面的 `python` 替换为该命令。

```powershell
$env:SHERPA_ONNX_ARCHIVE_DIR = (python tools/diarization/fetch-sherpa-archive.py).Trim()
if ($LASTEXITCODE -ne 0) { throw 'sherpa-onnx dependency verification failed' }

cargo build --release -p llama-helper -p diarize-helper
if ($LASTEXITCODE -ne 0) { throw 'Helper build failed' }

$helperTarget = ((rustc -vV | Select-String '^host:').ToString() -replace '^host:\s*', '').Trim()
New-Item -ItemType Directory -Force -Path frontend/src-tauri/binaries | Out-Null
Copy-Item -LiteralPath target/release/llama-helper.exe -Destination "frontend/src-tauri/binaries/llama-helper-$helperTarget.exe"
Copy-Item -LiteralPath target/release/diarize-helper.exe -Destination "frontend/src-tauri/binaries/diarize-helper-$helperTarget.exe"
```

### macOS / Linux / Bash

```bash
export SHERPA_ONNX_ARCHIVE_DIR="$(python3 tools/diarization/fetch-sherpa-archive.py)"
test -n "$SHERPA_ONNX_ARCHIVE_DIR" || exit 1
cargo build --release -p llama-helper -p diarize-helper || exit 1

helper_target="$(rustc -vV | sed -n 's/^host: //p')"
mkdir -p frontend/src-tauri/binaries
cp target/release/llama-helper "frontend/src-tauri/binaries/llama-helper-$helper_target"
cp target/release/diarize-helper "frontend/src-tauri/binaries/diarize-helper-$helper_target"
```

以上为 CPU 基线构建。若配置了 `CARGO_TARGET_DIR` 或显式交叉编译目标，需要相应调整复制来源。校验脚本只接受已固定校验值的目标平台，不支持的目标会报错。

`llama-helper` 的 CUDA、Vulkan、Metal 加速需要分别构建，例如 `cargo build --release -p llama-helper --features cuda`，再复制更新后的可执行文件。主应用的 GPU 参数不会自动重编译已经复制的辅助程序。

## 启动与打包

以下命令在 `frontend/` 中执行：

| 命令 | 用途 |
| --- | --- |
| `pnpm tauri:dev` | 自动检测 GPU 配置，启动桌面开发模式 |
| `pnpm tauri:dev:cpu` | 使用 CPU 配置启动主应用 |
| `pnpm tauri:dev:cuda` | 使用 CUDA 配置启动主应用，需相应 SDK |
| `pnpm tauri:dev:vulkan` | 使用 Vulkan 配置启动主应用，需相应 SDK |
| `pnpm dev` | 仅启动网页开发服务，端口 3118 |
| `pnpm build` | 构建前端静态输出 |
| `pnpm tauri:build` | 自动检测 GPU 配置并打包桌面应用 |
| `pnpm tauri:build:cpu` | 以 CPU 配置打包主应用 |

自动检测入口是 `scripts/tauri-auto.js`；可通过 `TAURI_GPU_FEATURE` 指定配置。更多 Metal、CoreML、OpenBLAS 等命令见 [package.json](package.json)。GPU 构建是否成功取决于对应平台和工具链。

Tauri 开发模式使用 `pnpm dev:tauri` 启动前端服务。浏览器直接访问网页只能用于部分界面开发，没有原生 IPC 时录音、存储和模型相关功能不可用。修改 Rust 核心后需要等待重新编译并加载新版应用。

打包包含平台资源、辅助程序和签名处理。正式发布另需签名配置；不要把本机开发构建视为已经完成签名和发布。输出路径以 Tauri 构建日志为准。

## 测试与检查

在 `frontend/` 中：

```bash
pnpm test
pnpm exec tsc --noEmit
pnpm lint
```

在仓库根目录中：

```bash
cargo test -p mityu --lib summary:: --no-default-features
cargo clippy -p mityu --lib --no-default-features
cargo fmt --all -- --check
```

Rust 主包仍名为 `mityu`。运行涉及 `diarize-helper` 的 workspace 构建或测试前，同样需要先校验 sherpa-onnx 依赖并设置 `SHERPA_ONNX_ARCHIVE_DIR`。不同平台的原生依赖与既有检查告警应分别排查。

本轮摘要、模板和语言选择器的定向前端测试：

```bash
pnpm exec vitest run src/components/LanguagePickerPopover.test.tsx src/components/MeetingDetails/SummaryTemplateEditor.test.tsx src/components/report/SpeakerTurns.test.tsx
```

单元测试不替代实际录音、音频回放或真实模型生成检查。

## 主要模块

| 目录 | 职责 |
| --- | --- |
| `src/app/` | Next.js 页面与布局 |
| `src/components/MeetingDetails/` | 会议转写、摘要、模板编辑与相关控件 |
| `src/hooks/meeting-details/` | 会议详情加载、摘要生成等状态逻辑 |
| `src/i18n/` | 界面翻译 |
| `src-tauri/src/audio/` | 音频采集和处理 |
| `src-tauri/src/summary/` | 摘要服务、模型调用、模板和语言处理 |
| `src-tauri/src/diarization/` | 说话人区分流程 |
| `src-tauri/src/database/` | 本地数据存储 |
| `src-tauri/templates/` | 内置与打包模板定义 |

模板 UI 使用 `api_list_templates`、`api_get_template` 与 `api_save_custom_template`。保存模板会校验字段与章节标题，使用 `custom_` 标识，并重新刷新可选模板列表。详细格式见 [模板说明](src-tauri/templates/README.md)。

## 常见问题

- **页面打开了，但功能报原生接口错误**：确认使用的是 `pnpm tauri:dev` 启动的桌面窗口，而不是独立浏览器页面。
- **找不到辅助程序**：检查 `src-tauri/binaries/` 中是否有与当前 target triple 匹配的文件。
- **选择中文后仍看到英文旧摘要**：语言变更作用于下一次生成，需要重新生成；模板章节标题保持原文。
- **只想本地运行**：使用内置 AI 或本机 Ollama，提前下载模型，无需启动仓库中的历史 Python `backend/`。
- **Speaker 数量或文字不准确**：区分结果是估计；同时发言的声音尚不能分别完整转写，需回听核对。
