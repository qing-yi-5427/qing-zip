# 参与 qing-zip

报告问题时，请附上「关于 qing-zip」中的版本、Windows 版本与架构、安装版或便携版、最短操作步骤及预期结果。请使用可公开分享的测试文件。安全问题请从私密渠道提交。[问题列表](https://github.com/qing-yi-5427/qing-zip/issues) · [安全问题报告](SECURITY.md)

## 本地检查

首次只需当前主线时，可以浅克隆。

```powershell
git clone --depth 1 https://github.com/qing-yi-5427/qing-zip.git
Set-Location qing-zip
```

从仓库根目录运行。构建需要 Windows、MSVC C++ Build Tools 和 Windows SDK、Rust stable（MSVC 工具链）、Python 3.10 或更高版本。前端测试使用 Node.js 22 或更高版本，无需安装 npm 依赖。

```powershell
python scripts/verify-runtime.py
python scripts/test-runtime-inventory.py
$frontendTests = @(Get-ChildItem -LiteralPath obz-gui/tests -Filter '*.cjs' -File | ForEach-Object FullName)
node --test @frontendTests
cargo check --locked --manifest-path obz-rs/Cargo.toml
cargo check --locked --manifest-path obz-gui/Cargo.toml
```

使用 `cargo audit --file obz-rs/Cargo.lock` 和 `cargo audit --file obz-gui/Cargo.lock` 检查两份锁文件。更新依赖时同步更新受影响的锁文件与第三方声明。修改打包逻辑时，请从新输出目录构建，并检查便携包、安装包、引擎源码及它们的 SHA-256。

常规 PR、推送和默认发布构建使用 Windows x64。需要 ARM64 整包时，在 GitHub Actions 手动运行 **Windows ARM64 native validation**。本地原生 ARM64 硬化程序可显式运行 `./scripts/build-hardened.ps1 -Target aarch64-pc-windows-msvc`。Explorer 组件的手动构建和签名验收需要在 **Explorer review package** 中选择架构 `arm64`。

## 测试与提交

在对应版本的源码检出中，`docs/PRODUCT_VISION.md` 和 `docs/OPTIMIZATION_ROADMAP.md` 说明选题依据。复现问题或比较候选版本时，查阅同目录的 `QUALITY_REPORT.md`、`TEST_MATRIX.md` 和 `PERFORMANCE_REPORT.md`。[源码仓库](https://github.com/qing-yi-5427/qing-zip)

- 在 PR 中写明可见变化、验证步骤和实际结果。处理归档修改、取消、解压和覆盖时，记录原包及输出的状态。
- 将逐轮截图、原始回执和打包产物保存在 PR 评论或 CI 的运行附件中，不提交到仓库。正式界面截图放在 `assets/screenshots/`，测试使用的小型固定样本放在 `scripts/fixtures/`。提交前运行 `git ls-files --cached --ignored --exclude-from=.gitignore`，有输出时从暂存区移出对应文件；再运行 `python scripts/check-tracked-artifacts.py origin/main`，它会拦截 `docs/` 下的截图、回执和日志，以及超过 200 KiB 的新文件。CI 会执行同样的检查。
- 本机验证优先使用普通临时文件。涉及 NTFS 备用数据流写入、事务、特殊文件系统边界的验证请在获准的可丢弃环境中执行。Windows 回归工作流保留独立的手动入口。
- 前端位于 `obz-gui/frontend`，桌面命令在 `obz-gui/src`，归档核心和 CLI 位于 `obz-rs`，Explorer 组件位于 `qing-shell`。组件许可证和发布清单位于 `licenses` 与 `runtime`。

贡献的原创代码适用项目 MIT 许可。引入或更新第三方文件时，请保留其许可证、来源和版本信息。[许可证](LICENSE) · [第三方声明](licenses/THIRD-PARTY-NOTICES.txt)
