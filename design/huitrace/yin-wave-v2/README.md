# 会迹 · 双声太极

保留太极的圆形和连续 S 曲线，以两组反色声波取代阴阳眼：黑中有白声、白中有黑声，表现交流双方的回应与记录。声波不跨越分界线，避免缩小后出现被切断的碎片。浅灰细边缘保持深浅桌面上的轮廓可见。

- `icon.svg`：1024px 矢量主稿，64 单位设计网格。
- `icon-small.svg`：24px 侧栏光学校正版。
- `icon-{size}.png`：16、20、24、32、40、48、64、96、128、256px。
- `huitrace-yin-wave.ico`：十帧 Windows 图标，**256px 必须在第一帧**。
- `preview.png`：新旧方案、原尺寸和放大像素对比。

## 导出

需要 Node.js、sharp 和项目已有的 Tauri CLI。`sharp` 可以来自 Node 模块搜索路径或 frontend 的 Next.js 依赖。

```powershell
node design/huitrace/yin-wave-v2/build-icons.cjs
node design/huitrace/yin-wave-v2/build-icons.cjs --apply
```

第一条只生成本目录资源；第二条同时更新 Tauri 各平台图标、网页 favicon、公用 PNG/SVG。旧版设计目录保留作对照，不再用 v1 脚本覆盖现用资源。

## 任务栏模糊的原因与修正

本机 `tauri-codegen-2.6.1/src/image.rs` 的 `CachedIcon::new_ico` 直接解码 `icon_dir.entries()[0]`，而原导出脚本按 16→256px 排列。这样运行中窗口和复用默认图标的托盘拿到的是 16px 位图，即使 ICO 含有高清帧仍会被放大。

新版将 256px 帧放在首位，保留全部小尺寸帧供 Windows 图标选择使用。小尺寸声波按像素对齐，以 4 倍采样再缩小处理圆角和曲线，替代旧脚本无抗锯齿的 `SetPixel` 写入。

图标嵌入可执行文件，资源更新需要重新构建并启动应用才会反映在运行中的任务栏。若已固定的快捷方式仍显示旧图标，可在新构建启动后取消固定，再重新固定。本次验证覆盖导出资源及 ICO 帧；不代表已验证安装后的 Windows 任务栏显示。
