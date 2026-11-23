# Windows 测试矩阵

2026-11-23 · 本地起点 `0cc51aa2379321361012f32515edb40b95e2ebf9` · qing-zip 0.12.23 · Windows 11 x64 26300。

`PASS` 已执行并通过。`FAIL` 已执行且未达到预期。`SKIP` 明确跳过且有记录。`BLOCKED` 缺少执行所需的设备、环境或授权。`NOT RUN` 本轮未执行。历史 CI 和本轮实测分开记录。

## 本轮验证

| 层级与入口 | 样本和断言 | 状态 |
| --- | --- | --- |
| 前端语法和 Node 回归 | 16 个 JS 语法检查，42 个测试文件，271 项通过。覆盖归档列表、键盘焦点、压缩参数、深度解压交互、密码本、任务重试、校验结果和右键转交队列。 | PASS |
| Python 运行时和发布脚本 | 校验 65 个 x64 运行时文件及 NanaZip 头文件。附加运行时清单、ARM64 包形状、修改 crate 源码来源和 ARM64 GUI 生命周期测试，共 8 项。合成普通 `.lnk` 文件的锁、内容更改和失败清理共 4 项。 | PASS |
| Rust 核心单元 | 32 项。列表 JSON 兼容、ZIP 时间字段、父目录任务顺序、运行时路径、LZMA 参数和报告，以及深度解压的计划、字节预算、分卷和发布状态。空间不足使用内存写入回调。 | PASS |
| Rust 深度解压集成 | 15 项经审阅的普通归档用例。多入口、错扩展名和无扩展名、ZIP/7z/RAR5、压缩 TAR、嵌套目录、分卷、选取、AES 密码续接、冲突重试和保留普通数字伴随文件。回收资格只读取会话报告，样本保留在临时目录。 | PASS |
| Rust CLI 集成 | 解压和测试的线程参数 1 项。校验工具的目录递归、输入顺序、不可读输入和打开写句柄共 3 项。 | PASS |
| Rust 引擎取消集成 | 取消后开始列包应返回取消。压缩普通 16 MiB 文件为 7z 的过程中触发取消，断言取消返回且目标包未发布。 | PASS（2 项） |
| x64 加固产物 | Release CLI、worker、桌面程序。三者均为 x64，PE 审计通过 ASLR、高熵 VA、DEP、CFG、CET 兼容和可写可执行节检查。 | PASS |
| x64 便携包实跑 | 解包后从包内 CLI 创建、列出、校验、解压 ZIP 与 7z。拒绝覆盖时比较已存在目标文件与原包字节。 | PASS |
| x64 便携包处理下载不完整的普通归档 | 将生成的 ZIP、7z 各截去后半段，二者的完整性检查和启用覆盖的解压均返回退出码 1 和错误输出。比较完整原包、截断副本、原有目标文件字节和目标目录项。 | PASS |
| 归档独立互读 | qing-zip ZIP ↔ Python `zipfile`。qing-zip 三卷固实 7z → 官方 7-Zip，官方 7-Zip 固实 7z → qing-zip。校验中文、空格、重名叶子、空文件、空目录、选取解压的跳过和覆盖、未选目标与源归档 SHA-256。使用便携包暂存目录的二进制。 | PASS |
| 常用格式三程序解压对照 | ZIP、7z、TAR、GZIP、BZ2、XZ、RAR5。qing-zip 0.12.23 加固便携、7-Zip 26.00 x64、Bandizip 7.36 Beta x64。比较相对文件名、输出文件 SHA-256 和每次读取后原包 SHA-256。 | PASS（7 格式 × 3 CLI） |
| WinRAR 对照 | 当前环境未安装 WinRAR。 | SKIP |
| 经典 RAR 普通读取 | RAR 1.5 单卷、RAR 3.00 两卷（`.rar` / `.r00`）。7-Zip、qing-zip、Bandizip 按上游明文哈希逐文件、逐卷对照。 | PASS（2 × 3） |
| 经典 RAR 参考样本分歧 | 上游 `split-store` 三卷。RARLAB 官方 UnRAR 7.30 beta 2 输出 21 字节预期文件并退出 0；qing-zip、7-Zip、Bandizip 返回错误。全部原卷哈希保全。 | FAIL（qing-zip 的分卷兼容性），PASS（UnRAR 参考工具） |
| x64 Shell 隔离验证 | M4 独立验证相同起始提交。CMake Release、MSVC 19.44、SDK 10.0.26100.0。参数往返、隔离 COM、普通文件选择均通过（3/3）。未签名包验证外置文件哈希及架构检查。 | PASS |
| 托管 x64 / ARM64 Shell 组件 | M4 提交 `9c27936`。x64 `windows-latest` 和 ARM64 `windows-11-arm` 均验证原生 Shell 组件和未签名稀疏 MSIX，各检查三个外置文件哈希、拒绝两种混用产物。 | PASS（独立 CI） |
| 前台桌面、DPI 和真实 Explorer | M3 跟踪前台应用与 DPI；正式安装后关联、旧式右键和真实拖出的验收需独立环境。 | NOT RUN（本矩阵） |
| Windows 11 现代右键 | 包签名、当前用户注册后菜单、卸载后菜单消失和升级保留关联。 | BLOCKED（可信签名和可安装验收环境） |
| Windows 10 与 Windows 11 ARM64 实机 | 最低系统版本、ARM64 GUI、解压、安装和 Explorer。 | BLOCKED（本轮主机为 Windows 11 x64） |
| 特殊文件系统、源归档回收和磁盘耗尽 | ADS/TxF、重解析及链接、真实回收站、真实盘满，另在获准的隔离环境验证。合成空间不足的预算模型已列在 Rust 单元行。 | BLOCKED（未启用隔离环境） |
| 扩展格式、嵌套编辑及其他损坏模式 | 尚待扩大独立对照。包括 ZIPX、传统加密 ZIP、ALZ/EGG、ISO、嵌套编辑写回、其他损坏类型与 ZIP 修复。 | NOT RUN |
| GUI 取消、退出和重启后的恢复 | 已覆盖引擎压缩取消、内存取消模型和同一会话的密码续接/发布重试。GUI 真实取消、进行中解压的取消、程序进程中断及重启后恢复尚待验收。 | NOT RUN（上述端到端场景） |
| 异常路径和数量预算 | 深度、数量和字节计费由内存用例验证，也执行了正常样本的选取预算。异常路径及大量嵌套、超文件数的真实归档场景尚待验证。 | NOT RUN（实包边界） |
| Rust 格式检查 | `obz-rs` 与 `obz-gui` 均有既有 rustfmt 差异，例如 `obz-rs/build.rs` 和 `obz-gui/src/archive_listing.rs`。 | FAIL |

产物 `qing-zip-0.12.23-portable.zip` 为 33,340,323 字节，SHA-256 为 `61918ca1723598776e79de5ed0f677336ccc2b5c22e5951d49f291dd48e8ab09`。四类普通归档与引擎基准见 [性能基线](PERFORMANCE_REPORT.md)。

常用格式对照的细节见 [质量报告](QUALITY_REPORT.md)。Shell 包和原生接口的证据见 [M4 验证记录](https://github.com/qing-yi-5427/qing-zip/pull/1) 与 [原生 x64 / ARM64 托管运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45096359747)。

## 复跑命令

在仓库根目录执行。Node 24.14.0、Python 3.14.0 与 Rust 1.99.0 x64 MSVC 为本轮宿主工具。

```powershell
Get-ChildItem obz-gui/frontend -Filter *.js -File | ForEach-Object {
    node --check $_.FullName
    if ($LASTEXITCODE) { throw "Frontend syntax failed: $($_.Name)" }
}
$tests = @(Get-ChildItem obz-gui/tests -Filter *.cjs -File | ForEach-Object FullName)
node --test @tests

python scripts/verify-runtime.py
python scripts/test-runtime-inventory.py
python scripts/test-arm64-package-only.py
python scripts/test-modified-crate-sources.py
python scripts/test-arm-gui-lifecycle.py
$shortcutReceipt = Join-Path $env:TEMP ('qing-shortcut-' + [guid]::NewGuid() + '.json')
python scripts/test-arm64-shortcut-cleanup.py --receipt $shortcutReceipt
```

Rust `--lib` 模块筛选仅用于已审阅的五个模块。其余用例按名称和 `--exact` 选择。构建命令单独运行，`-RunTests` 会启动完整 Rust 测试集。

```powershell
$target = Join-Path $PWD 'target/quality-rust'
$core = @('test', '--locked', '--manifest-path', 'obz-rs/Cargo.toml', '--target-dir', $target)
$exact = @(
    'engine::staging_tests::technical_listing_preserves_mixed_line_endings_and_metadata'
    'engine::staging_tests::technical_listing_created_accessed_and_legacy_payload'
    'operations::tests::listing_response_matches_legacy_json_including_key_order'
    'engine::selected_worker::zip_listing::tests::zip_member_times_distinct_values_and_precision'
    'engine::selected_worker::zip_listing::tests::zip_member_times_absent_values_stay_empty'
    'engine::parent_round_robin_preserves_every_index_and_parent_order'
    'engine::release_helpers_use_only_the_bundled_runtime'
)
foreach ($case in $exact) {
    $result = cargo @core --lib $case -- --exact 2>&1
    $result
    if ($LASTEXITCODE -or !(($result | Out-String) -match 'test result: ok\. 1 passed; 0 failed;')) {
        throw "Expected one passing test: $case"
    }
}
$modules = @(
    'engine::benchmark::tests'
    'deep_extract_plan::tests'
    'deep_stream_copy::tests'
    'deep_publication_state::tests'
    'deep_volume_plan::tests'
)
foreach ($module in $modules) {
    $result = cargo @core --lib $module 2>&1
    $result
    if ($LASTEXITCODE -or !(($result | Out-String) -match 'test result: ok\. [1-9]\d* passed; 0 failed;')) {
        throw "Expected passing tests: $module"
    }
}
```

```powershell
$deep = @(
    'batch_defaults_to_each_source_parent_and_uses_one_dedicated_output'
    'prefixed_wrong_extension_chain_preserves_final_photos_directory'
    'mixed_contents_apply_smart_child_folder_rule'
    'selected_outputs_only_are_expanded_and_budgeted'
    'visible_header_encryption_reuses_session_password_and_resume_keeps_done_work'
    'publication_retry_copies_only_unresolved_files'
    'directory_merge_retry_preserves_mapping_without_duplicate_folders'
    'raw_gzip_tar_stream_is_bounded_and_unpacked_as_another_level'
    'real_sevenz_inside_prefixed_jpg_then_zip_preserves_terminal_folder'
    'complete_split_sevenz_selection_resolves_to_one_archive'
    'resumed_password_is_reused_for_two_paused_sibling_archives'
    'missing_extension_and_other_compressed_tar_formats'
    'ordinary_rar5_nested_archive_uses_existing_decoder'
    'ordinary_numeric_sibling_is_retained_without_false_partial_or_wrapper_loss'
    'mixed_zip_public_selection_works_without_a_password'
)
foreach ($case in $deep) {
    $result = cargo @core --test deep_extract $case -- --exact 2>&1
    $result
    if ($LASTEXITCODE -or !(($result | Out-String) -match 'test result: ok\. 1 passed; 0 failed;')) {
        throw "Expected one passing deep extraction test: $case"
    }
}
cargo @core --test extraction_threads
cargo @core --test checksum_files
cargo @core --test engine_integration engine_cancel_before_operation -- --exact
cargo @core --test engine_integration engine_cancels_running_compression_without_output -- --exact
cargo fmt --check --manifest-path obz-rs/Cargo.toml
cargo fmt --check --manifest-path obz-gui/Cargo.toml
```

```powershell
$build = ./scripts/build-hardened.ps1
$output = Join-Path $PWD ('dist/quality-' + [guid]::NewGuid())
python scripts/package.py --binary-directory $build.BinaryDirectory --output-directory $output
python scripts/check-public-package.py "$output/qing-zip-0.12.23-portable.zip"
python scripts/check-archive-interop.py --cli "$output/qing-zip-0.12.23-portable/obz.exe"
```

## 公开 CI 历史记录

以下记录对应早一版公开提交 `517ef21a6dcac2cee9cd438ee3053597220e3436`，平台覆盖仍有参考价值。

| CI | 已通过 | 跳过 |
| --- | --- | --- |
| [Windows x64 regression · 45003050224](https://github.com/qing-yi-5427/qing-zip/actions/runs/45003050224) | 已发布运行时校验、有限前端/纯 Rust、深度解压正常样本、加固 x64 便携包。 | 完整前端、原生 Explorer、核心扩展套件、原生链接与桌面回归、注册后 Explorer 测试均为该运行的 `SKIP`。 |
| [Windows ARM64 native validation · 45003050222](https://github.com/qing-yi-5427/qing-zip/actions/runs/45003050222) | 原生运行时和工具、选定纯回归、ARM64 便携包和安装包构建、源码包。 | 原生 CLI/分卷、ADS/链接、经典菜单、安装及卸载的实跑为 `SKIP`。 |
| [Windows EFS regression · 45003050182](https://github.com/qing-yi-5427/qing-zip/actions/runs/45003050182) | 运行状态为 `SKIP`。 | EFS 真实操作。 |
