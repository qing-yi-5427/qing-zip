# 参与 qing-zip

报告问题时，请附上「关于 qing-zip」中的版本、Windows 版本与架构、安装版或便携版、最短操作步骤及预期结果。请使用可公开分享的测试文件。安全问题请从私密渠道提交。[问题列表](https://github.com/qing-yi-5427/qing-zip/issues) · [安全问题报告](SECURITY.md)

## 本地检查

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

## 测试与提交

选题前可查阅[产品定位](docs/PRODUCT_VISION.md)与[优化路线](docs/OPTIMIZATION_ROADMAP.md)。复现问题或比较候选版本时，使用[质量报告](docs/QUALITY_REPORT.md)、[测试矩阵](docs/TEST_MATRIX.md)和[性能报告](docs/PERFORMANCE_REPORT.md)里的基线和命令。

- 在 PR 中写明可见变化、验证步骤和实际结果。处理归档修改、取消、解压和覆盖时，记录原包及输出的状态。
- 本机验证优先使用普通临时文件。涉及 NTFS 备用数据流写入、事务、特殊文件系统边界的验证请在获准的可丢弃环境中执行。Windows 回归工作流保留独立的手动入口。
- 前端位于 `obz-gui/frontend`，桌面命令在 `obz-gui/src`，归档核心和 CLI 位于 `obz-rs`，Explorer 组件位于 `qing-shell`。组件许可证和发布清单位于 `licenses` 与 `runtime`。

贡献的原创代码适用项目 MIT 许可。引入或更新第三方文件时，请保留其许可证、来源和版本信息。[许可证](LICENSE) · [第三方声明](licenses/THIRD-PARTY-NOTICES.txt)
