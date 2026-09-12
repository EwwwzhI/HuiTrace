# 会迹 · 太极声流图标

视觉核心是旋转后的太极关系：黑白两极代表对话双方，中央五段声波在黑白区域交界处自动反色，表达语音在交流中的流动、记录与留痕。

最终轮廓是标准圆形，不是圆角方块；圆外区域为真实透明 Alpha。外圈使用深色框，在浅色与深色桌面背景上都能保持边界。大尺寸采用五段宽间距声波；16px 光学校正版简化为三段，20–64px 分别使用独立、整数像素对齐的五段声波，避免相邻声波被 Windows 抗锯齿合并。

内置 image_gen 用于探索和确定太极、对话与反色声波的构图方向；生产文件改为纯矢量 SVG，以消除生成图常见的假透明棋盘格、边缘畸变和小尺寸模糊。

## 文件

- `icon.svg`：可编辑的主矢量源文件。
- `huitrace-yin-wave.png`：512px RGBA 预览与应用源图。
- `huitrace-yin-wave.ico`：包含 16/20/24/32/40/48/64/96/128/256px 共 10 帧的 Windows ICO，其中 16–64px 是独立绘制的任务栏母版。
- `icon-*.png`：各常用尺寸的透明 PNG。
- `taskbar-preview.png`：将 16–64px 原始像素等比例放大的检查图。

## 重新导出

从仓库根目录运行：

```powershell
& ./design/huitrace/yin-wave-v1/build-icons.ps1
```

脚本依赖 `frontend/node_modules` 中已经安装的 Tauri CLI。

构建脚本同时把本方案接入以下位置：Tauri 全平台图标、Windows 多尺寸 ICO、Next.js favicon、侧栏/关于页使用的公共 SVG，以及旧版兼容 PNG 入口。
