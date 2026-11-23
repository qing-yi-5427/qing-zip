# Windows 11 ARM64 便携目录迁移与现代菜单注销

一次性 GitHub 托管 Windows 11 ARM64 上使用同一便携应用、签名后的稀疏 MSIX 和一次性测试证书。包全名为 `QingYi.QingZip.Shell_0.12.23.0_arm64__akcd4eewvgq7r`，通过现有 `Register-Package.ps1` 和 `Unregister-Package.ps1` 切换当前用户注册。登记位置从 Windows Runtime 的 `Package.UserExternalPath` 和 `Package.EffectiveExternalPath` 读取。 [托管验收与完整截图](https://github.com/qing-yi-5427/qing-zip/actions/runs/45132964898) · [微软 UserExternalPath](https://learn.microsoft.com/uwp/api/windows.applicationmodel.package.userexternalpath) · [微软 EffectiveExternalPath](https://learn.microsoft.com/uwp/api/windows.applicationmodel.package.effectiveexternalpath)

| 操作 | Windows 注册记录 | 实际 Explorer |
| --- | --- | --- |
| 在旧目录注册，启用该份应用的打开菜单设置 | 用户和有效外置位置均为旧目录的 `shell` | — |
| 保留旧注册和旧设置，仅移动文件 | WinRT 和当前用户目录仍有同一包，两个路径仍指已不存在的旧 `shell` | 新窗口显示标准右键菜单，一级 qing-zip 分组未出现 |
| 回位、注销、移动，再在新目录注册同一签名包 | 移动前后均确认未注册。完成重注册后，两个外置位置均改为新目录的 `shell` | 新窗口由一级 qing-zip 分组点「解压到此处」，观察到新路径下的应用进程并得到两个正确成员 |
| 保留新路径的应用打开菜单设置，单独注销包 | WinRT 查不到原包，当前用户 Appx 目录无此包 | 新窗口的标准右键菜单保留「Show more options」，一级 qing-zip 分组消失 |

便携应用及所加外置组件与测试哨兵共 **731 个文件**，迁移后和最终注销清理后都逐一核对 SHA-256。三次 Explorer 观察分别使用独立目录，源 ZIP 和邻接哨兵的 SHA-256 均保持一致，「解压到此处」只新增所列两个成员。最终移除了本次试用的菜单设置和测试证书。各场景关闭自己打开的窗口。 [迁移与文件哈希回执](portable-lifecycle.json) · [新路径解压](lifecycle-ui-relocated.json) · [只移动观察](lifecycle-ui-stale.json) · [注销后新窗口](lifecycle-ui-unregistered.json) · [清理回执](setup.json)

[实际解压完成截图](lifecycle-relocated-extracted.png)

源便携包对应 `fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6`，ARM64 便携 ZIP SHA-256 为 `cc0e06dafef5d4caf7f8905ee87c5396f8e3f022c03c2a6f382dada4321f3bbe`。[原始包构建](https://github.com/qing-yi-5427/qing-zip/actions/runs/45128040730)

在本验收分支调度 `shell-package.yml`，同时启用 `hosted_explorer` 和 `hosted_portable_lifecycle`，可复跑该签名测试流程。证据覆盖当前用户的 Windows 11 ARM64 稀疏包生命周期。Windows 10、现有已打开窗口里的菜单缓存及正式安装器分别需要验收。
