# Phase 2D.2a-final — P0 Integrity Closure 开发报告

日期：2026-09-14。范围仅包含 P0-1 / P0-2 / P0-3。

实现和合成回归已完成；真实会议人工双遍 smoke 尚未完成，因此暂不宣布
`Formal Representative Ground-Truth Collection Ready`，也不开始正式规模标注。

1. **最终 commit**：本报告随实现一起提交；具体 SHA 见交付回复，或运行
   `git log -1 --format=%H -- docs/SHORT_TURN_PHASE_2D_2A_FINAL_REPORT.md`。
   分支为 `codex/phase-2d-2a-p0-integrity`，本轮不推送远端。
   开发前以 `git ls-remote` 验证远端 main 基线为
   `d6cc5d51f19c4c9d62261ea6d0533981fa13ad3e`。

2. **修改文件**：
   - `frontend/src-tauri/src/evaluation/annotation_workspace.rs`
   - `frontend/src-tauri/src/evaluation/dataset.rs`
   - `frontend/src-tauri/src/bin/short_turn_benchmark.rs`
   - `frontend/src-tauri/tests/annotation_integrity_workflow.rs`
   - `frontend/src/app/dev/short-turn-annotation/page.tsx`
   - `frontend/src/app/dev/short-turn-annotation/page.i18n-theme.test.tsx`
   - `frontend/src/lib/annotationIntegrity.ts`
   - `frontend/src/lib/annotationIntegrity.test.ts`
   - `frontend/src/i18n/en.json` / `zh-CN.json`（仅新增完整性提示文案）
   - `docs/SHORT_TURN_ANNOTATION_WORKSPACE.md`
   - `docs/SHORT_TURN_PHASE_2D.md`
   - 本报告。

3. **P0-1 修复方式**：`completeWindow()` 在更改状态前检查 pending 并显示数量，
   不自动确认任何事件。现有 `save_workspace_inner()` 作为后端 authority，
   在写入 draft/session 前重新验证所有已完成窗口；无需改变 autosave 架构。
   Review 同时拒绝 `review_pending` 和遗留 `pending`。

4. **当前 Window 判断**：共享时间相交语义为
   `event.start_ms < window.source_end_ms && event.end_ms > window.source_start_ms`。
   不依赖创建窗口 ID，恰好接触边界不算相交；跨两个 viewport 的 pending 会阻止两个窗口。
   编辑/undo 恢复 pending 时重新打开受影响窗口；仅 Review pending 时保留已完成的 Blind 状态。

5. **Review entry guard**：除校验所有预期 Blind ID 外，检查整个 draft 的 pending 数量。
   旧 session 即使全部标成 `reviewed_blind`，仍会被拒绝。由于直接 QA-mode load
   也会返回 Review evidence，同样应用此 guard；未做 Qa 模式清理。现有最终 QA 确认检查保留。

6. **P0-2 后端 Export gate**：先读取两份 window 文件，检查 Blind/Review 完成，
   再进行 QA、artifact identity、当前 draft/session 与磁盘已保存状态一致性检查，
   最后构造和校验合并 manifest。前端增加 Review 进度和未完成时的禁用提示，
   原有 QA revision / saved revision 条件保留。

7. **Completion 计算**：按 window 文件中去重后的预期 ID 判断，不按 session key 数量。
   Blind 接受 `reviewed_blind` / `reviewed_second_pass`；Review 只接受
   `reviewed_second_pass`。空窗口集、缺失 ID 均失败。错误文本包含 completed、total、remaining。

8. **失败导出与 manifest**：所有进度、QA、保存一致性、artifact、JSON 解析和 dataset
   校验失败均发生在写入前；回归测试按字节比较 meeting/root 两份 manifest。
   root 替换失败时恢复 meeting 原内容。两个文件仍不是崩溃原子事务；突然终止或存储介质
   连回滚都无法完成时不能保证恢复，不声称已解决这类底层存储故障。

9. **Speaker Reference 定义**：明确的 `ordinary_speech_control`，时长 **>1200 ms**，
   非空已知 speaker，`annotation_uncertain=false`，无 `overlap` tag。
   不因 handoff/embedded 单独排除；采集时建议选择至少 2 秒清晰语音。

10. **GroundTruthKind 原始语义**：共享 enum 放在 `dataset.rs`；ManifestRow 直接
    反序列化为该 enum。仅 production 比较时显式转换到 SegmentKind。
    兼容旧 `speech` 标签的既有 coverage 解释，但绝不据时长将其变为 reference。
    annotation/dataset/benchmark 的 label 解释复用共享定义。

11. **Alignment 输入**：每场会议只从合格 reference 收集 GT speaker 和 overlap 权重，
    对生产 `raw_diarizer_turns` 累积 temporal overlap，执行原 maximum-weight one-to-one
    assignment，在短事件比较前冻结。没有改生产推理、阈值或 artifact schema。

12. **禁止输入**：`short_speech`、`backchannel`、`noise`、`non_speech_vocalization`、
    legacy `speech`、uncertain/overlap/无 speaker/≤1200 ms reference 均不参与 mapping。
    即使大量错误短事件的总时长超过 reference，也无法改变 mapping。

13. **无 Reference Speaker**：映射为 unavailable（内部比较 speaker 为 None），
    不进入 individual attribution 分母；混合映射/未映射的 ambiguous speaker set 也不评分。
    没有 all-GT fallback。原始 GT 的 representative coverage 在 alignment 前计算。

14. **Coverage 报告**：benchmark JSON 的 `speaker_alignment[meeting_id]` 包含
    `mapped_speakers`、`total_gt_speakers`、`unmapped_gt_speakers`、
    `reference_intervals`、`reference_duration_ms` 和冻结后的 `mapping`。
    保留既有 assignment 的最多 20 个生产 cluster 限制，超出时显式显示 unmapped。
    Blind UI 没有新增生产 cluster identity。

15. **新增 Regression Tests**：窗口 pending/边界/相交双窗口；后端保存拒绝且不改 session；
    历史错误状态 Review 拒绝与确认后成功；Review 50%、199/200、missing status、
    QA pending、unsaved、原 manifest 损坏时拒绝且保持两份 manifest；完整导出成功；
    Review pending 中途保存后重新加载；交换 cluster；错误短事件不改变 mapping；
    无 reference/uncertain/overlap/legacy 排除；多 reference 累积；2/3 speaker coverage
    与分母；原始 label 保留；前端显式确认 gate 与 QA 通过但 partial Review 禁用 Export。

16. **实际执行的测试与构建**（均在 Windows 本地；输出保存在未跟踪的 `output/p0-*`）：

    ```text
    cargo fmt --all --check
    cargo check
    cargo test
    cargo test -p huitrace --lib annotation_workspace
    cargo test -p huitrace --bin short_turn_benchmark
    cargo test -p huitrace --test annotation_integrity_workflow
    pnpm --dir frontend test
    pnpm --dir frontend exec tsc --noEmit
    pnpm --dir frontend lint
    pnpm --dir frontend build
    git diff --check
    ```

    前端：43 files / 269 tests 通过；TypeScript、lint、build 通过。
    annotation_workspace：10 tests；benchmark：18 tests；整链集成：1 test 通过。
    Rust 全量结果见最终交付回复；编译与 lint 存在原有 warning，不记为零 warning。
    最后一轮普通 shell 的全量重链接曾因 `LNK1104: cannot open file 'msvcrt.lib'`
    失败。随后仅在测试进程设置 `LIB`，加入本机 MSVC 14.44.35207 的 `lib/x64`
    及 Windows SDK 10.0.26100.0 的 `ucrt/x64`、`um/x64` 后再次运行 `cargo test`。
    未改变仓库依赖或编译配置；保留失败日志和重跑日志，避免将首次失败写成成功。
    补齐库路径后，又遇到正在运行的开发应用锁住 `target/debug/huitrace.exe`。
    将该旧构建文件同目录重命名保留为 `huitrace.running-p0-42316.exe` 后重跑，
    没有关闭应用进程；该文件位于被忽略的构建目录，不纳入提交。

17. **Synthetic workflow**：A/B/C/D 通过实际 backend initialize/save/load/QA/export；
    E 验证错误 prediction 归为 `SpeakerAttributionError`，mapping 不变。
    另有生成 5 秒 WAV 的 CLI 集成测试，从 `short_turn_export` 经双遍确认与 Export
    调用 Dataset Check 和 Frozen Replay，返回 2/2 mapped、2 references、4000 ms，
    且 artifact 字节未变。全部使用明确的 synthetic 标签，不视作人工 GT 或真实 accuracy。

18. **真实 Smoke workflow**：未完成。找到本地一段约 611 秒的现有导入录音；
    项目 dataset 目录只有 example 文件，未发现对应已导出 artifact 和人工双遍 draft。
    对现有应用数据库的只读普通 SQLite 查询返回 `file is not a database`，没有尝试修改数据库。
    不能据此核实完整生产快照或 speaker 数量，也没有伪造人工确认。
    仍需在真实桌面环境完成 Production → artifact export → 10–15 分钟、2–4 speaker
    独立人工 Blind/Review 100% → QA/Saved/Export → Frozen Replay 的正式 smoke。

19. **Remote CI**：`Remote CI not observed`。仓库有 GitHub workflows，但本轮为本地提交；
    查询 GitHub Actions API 返回 403 rate limit exceeded，未观察到可验证的远端运行。
    不声称 CI passed。

20. **Remaining P1/P2**：Window JSON SHA binding、重新初始化保护、media duration pre-probe、
    Qa-mode cleanup、文件选择器、minimap/theme polish、后端错误 i18n、布局/tooltips、
    representative gate tuning 均未开展。真实会议 smoke 属于尚未完成的验收项，不能以 P1/P2
    名义绕过。正式采集就绪状态在该项完成前保持待验收。
