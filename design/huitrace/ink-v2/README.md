# 会声成迹 · 正式接入版

采用用户认可的相向墨笔、中央声纹、朱砂印点。内置 image_gen 精修了主体比例和留白，底板改为满幅宣纸底，去除厚重高光。原始视觉素材是 artwork.png。

icon.svg 是内嵌位图的生产包装，使用标准圆角裁切形成真实透明边缘；它不是可编辑笔画的矢量重绘。主体不经过背景色抠图，不会残留棋盘格。huitrace-ink.png 是应用源 PNG，huitrace-ink.ico 包含 16/20/24/32/40/48/64/96/128/256 像素。

从仓库根运行 `& ./design/huitrace/ink-v2/build-icons.ps1` 可重新生成全部平台资源、ICO、favicon 和 public 图片。需要已安装 frontend 的依赖。export.ps1 仅进行尺寸和格式导出。generated/ 为 Tauri 导出的中间产物，不纳入版本控制。

接入：frontend/src/components/Logo.tsx 和 About.tsx 使用新的 huitrace-icon-ink.png 文件名；移除额外 CSS 圆角避免二次裁切。兼容入口 huitrace-icon.png、icon_128x128.png、icon_32x32@2x.png 同步更新。桌面 bundle 沿用 icons/icon.png、icon.ico、icon.icns，托盘沿用 default_window_icon，因此无需改动 Rust 行为。旧蓝色源图保留作为历史设计。

验证：目视检查 512px 和 32px；10 个 ICO 帧可解码且角落 alpha 为 0；应用 ICO 与 favicon 哈希一致；TypeScript --noEmit --incremental false 和组件 diff --check 通过。当前未构建或安装桌面二进制，也未声称已验证 Windows 实际任务栏；多尺寸减少部分缩放比例的插值，但不能消除所有系统缩放与图标缓存影响。

## 精修提示词（内置 image_gen）

Refine this approved HuiTrace ink app icon for final production. Preserve precisely the two facing dark ink conversation forms, central three sound bars, small red seal at bottom right, and ivory paper aesthetic. Clean up overly fine dry-brush spray, slightly widen the narrow ivory negative-space gaps around the central sound bars so it reads at 24 pixels. Retain authentic brush texture and silhouette. Critical canvas change for production masking: REMOVE the rounded tile boundary, its bevel, its shadow and ALL transparency. Extend the SAME warm ivory paper background uniformly edge-to-edge across the ENTIRE square canvas, including every corner. Make the design a completely full-bleed opaque square image with no frame, no rounded corners, no outside region, no checkerboard, no padding. Artwork emblem occupies 78% of square. Keep polished subtle paper texture, no puffy cushion effect. One icon only, no text.
