# HuiTrace 品牌初稿

## 当前使用：太极声流版

当前应用图标采用 `yin-wave-v1`：黑白太极表示对话双方，中央声波穿过分界并反色，圆外是真实透明区域。主源文件、Windows 多尺寸 ICO 和可复现接入脚本见 [yin-wave-v1/README.md](yin-wave-v1/README.md)。

## 历史版本：水墨版

已接入用户认可的「会声成迹」水墨图标，源素材、生产包装、可复现导出脚本及验证记录见 [ink-v2/README.md](ink-v2/README.md)。后续请运行 ink-v2/build-icons.ps1 更新图标。下文蓝色版本为历史记录。

初稿方向：深青色 + 米白 H，连接线表达声音到文字的轨迹，薄荷绿圆点表达可追溯来源。

主图：frontend/public/huitrace-icon.png。通过 frontend 目录下的 `pnpm exec tauri icon public/huitrace-icon.png` 生成桌面各尺寸 PNG、ICO、ICNS；favicon 使用生成的 icon.ico。

已接入窗口/安装产品名称、托盘、通知、侧栏、关于页、引导、导出作者和网页标题。

兼容性：暂时保留 com.bluedev.mityu 应用标识、Rust 包名、数据库/录音目录、恢复缓存与导出 schema，以继续读取既有数据。保留上游版权和来源。旧设计文件作为历史参考保留。

更新：清空上游更新地址，暂停检查并禁用关于页/托盘更新按钮，等待配置 HuiTrace 自己的发布地址及签名。

边界：原有 Mityu Pro 的商业授权和支付文案保持原产品名，未把上游付款入口冒充为 HuiTrace。此次没有改变授权机制，也没有改造 landing 官网或发布工作流。

桌面图标与产品名称需重新构建并安装才能替换已安装版本；本次资产为可迭代初稿。

## 蓝色修订

图标颜色改为与应用 primary 相配的蓝色、白色 H、浅蓝节点。旧图的灰色角落是生成图片烘焙的棋盘格，并非透明背景。

正式源文件改为 design/huitrace/icon-blue.svg：内嵌蓝色位图，并用圆角 clipPath 生成真实透明边缘。请从 frontend 运行 `pnpm exec tauri icon ../design/huitrace/icon-blue.svg`，再同步生成的 icon.png 到 public/huitrace-icon-blue.png 与 public/huitrace-icon.png，icon.ico 到 src/app/favicon.ico，以及 128x128.png、64x64.png 到 public 对应尺寸文件。

侧栏与关于页使用新文件名 huitrace-icon-blue.png 避免旧缓存，并统一圆角。验证源 PNG 与 32/128 像素导出图的四角 alpha 均为 0；TypeScript 检查通过。
