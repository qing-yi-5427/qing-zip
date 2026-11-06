# Windows 11 现代右键入口

这是独立的 `IExplorerCommand` 组件和稀疏 MSIX 打包流程。x64 组件已构建，隔离接口测试与应用队列测试通过；尚未签名、安装或验证资源管理器主菜单，不能计入已交付的现代菜单功能。

四个入口通过应用协议执行，每次最多 128 项：

| 入口 | 参数 | 行为 |
| --- | --- | --- |
| 用 qing-zip 打开 | `<路径…>` | 在现有窗口中新开标签 |
| 用 qing-zip 压缩 | `--compress-folder <目录…>` | 打开现有新建压缩对话框 |
| 智能解压 | `--smart-extract <档案…>` | 复用现有同名首级目录判断，自动选择是否创建外层文件夹 |
| 解压到此处 | `--extract-here <档案…>` | 以源文件父目录为目标，明确关闭本次智能目录规则 |

已有单实例机制负责转交到当前窗口。DLL 不执行压缩或解压，不经过命令解释器，不修改文件关联。解压任务使用现有可取消任务页，继承重名处理、完成后打开目录、全部成功后关闭窗口和符合条件时回收源文件的偏好；不会改变用户保存的智能解压默认值。回收仍由原有完整解压、单卷、源身份和非跳过检查保护。不支持回收的格式或跳过策略保留源文件。

多项依次执行，失败或取消暂停剩余队列，用户可继续或清空；有其他待打开/待压缩/待解压任务时不自动关闭窗口。已打开同一档案时复用其密码和编码，其他档案不会继承无关标签的密码。失败任务保留在队首，可直接重试或通过“输入密码重试”为当前档案提供密码；密码只保存在本次队列项内存中，成功或清空后移除，不写入设置或其他标签。

显示状态读取现有两个设置键的命令值，并要求它们指向同一安装目录。关闭现有右键设置后，现代入口也隐藏；旧的“显示更多选项”入口保留。直接压缩普通文件、指定格式快速压缩、测试档案和校验子菜单尚未提供。多个文件动作由 Windows 按同一应用归组，实际显示布局需在签名安装后验收。

安装布局：

```text
qing-zip.exe
shell/
  qing-shell.dll
  qing-shell-host.exe
  Assets/AppIcon.png
```

MSIX 的 `ExternalLocation` 只指向 `shell` 子目录。它仅声明 COM 和现代菜单，未声明文件关联或默认应用，不碰 `UserChoice`。独立 host 的嵌入身份与 MSIX 的 Name、Publisher、Application Id 必须匹配。包内保存外置 DLL、host 和图标的 SHA-256；注册前同时验证受信包签名、产品身份与外置文件哈希。

构建和打包分成四步，可分别审查：

1. 运行 `scripts/msix/Build-Component.ps1`，生成 DLL、host，并运行真实 Windows 参数启动、隔离 COM、文件系统选择测试；需要 MSVC、Windows SDK 和 CMake。
2. 运行 `Prepare-Package.ps1`，显式提供组件目录、新输出目录、四段版本及签名证书的精确 Subject。它只生成带身份的外置组件和未签名 MSIX。
3. 使用 `Sign-Package.ps1` 和当前用户已有的受信代码签名证书签名，或在现有可信签名服务中签名。脚本不会创建、导入或信任证书。
4. 安装器未来可在复制组件后调用 `Register-Package.ps1`，在卸载文件之前调用 `Unregister-Package.ps1`。两者只针对当前用户和匹配发布者，支持 `-WhatIf`。目前没有接入现有安装器。

外部分发依赖受信代码签名证书或签名服务；本地开发证书也需要用户明确选择信任。缺少这些条件时，能交付未签名构建产物，不能承诺用户安装后出现现代菜单。不能用传统注册表菜单、关闭 Windows 11 新菜单或修改系统信任来冒充完成。

已执行的验证：MSVC x64 `/W4 /WX` 构建；三个原生测试程序；Rust 单实例与队列 7 项；`node --test obz-gui/tests/shell-queue.cjs` 的 17 项队列模型测试；未签名稀疏 MSIX 打包。原生测试只调用接口和专用参数记录进程，没有操作资源管理器窗口；Node 模型测试不等于真实 GUI 点击。另已通过浏览器点击验证实际前端的失败保留、密码对话框和重试完成状态；这部分使用明确的测试后端，仍不代表安装后的原生桌面或 Explorer 验收。

后续验收必须包括：Windows 11 默认右键中直接可见、点击后原实例接收路径、ZIP 被识别为文件、中文/空格/尾斜杠目录、多选、设置开关同步、便携版抢占设置时旧安装入口隐藏、路径不存在、虚拟项和重解析点拒绝、卸载后入口消失、重装与升级保持关联不变。需要重载资源管理器时应由用户知情操作，不强杀其窗口。

官方依据：

- [Microsoft：IExplorerCommand 与现代菜单注册](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/integrate-packaged-app-with-file-explorer)
- [Microsoft：外置目录包身份、签名和安装协议](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/grant-identity-to-nonpackaged-apps)
- [Microsoft：GetState 异步回调规则](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iexplorercommand-getstate)
- [NanaZip 7.0.1882.0 包清单](https://github.com/M2Team/NanaZip/blob/7.0.1882.0/NanaZipPackage/Package.appxmanifest)
- [NanaZip 7.0.1882.0 现代菜单实现](https://github.com/M2Team/NanaZip/blob/7.0.1882.0/NanaZip.UI.Modern/NanaZip.ShellExtension.cpp)
- [PowerToys：将外置包目录单独隔离的原因](https://github.com/microsoft/PowerToys/blob/main/src/PackageIdentity/readme.md)
