# Windows 测试矩阵

2026-11-23 · 本地起点 `0cc51aa2379321361012f32515edb40b95e2ebf9` · qing-zip 0.12.23 · Windows 11 x64 26300。

`PASS` 已执行并通过。`FAIL` 已执行且未达到预期。`SKIP` 明确跳过且有记录。`BLOCKED` 缺少执行所需的设备、环境或授权。`NOT RUN` 本轮未执行。历史 CI 和本轮实测分开记录。

## 已安装主线 9c9258c

| 实际操作 | 结果 |
| --- | --- |
| 本地 NTFS、随包 NanaZip、普通单卷明文 ZIP，写入期间强退 | PASS 原包保全、重试与并发任务。改名、删除各三轮均回收原包旁的候选。新增成员三轮 FAIL 候选回收。[三种操作的原始回执](verification/round6/installed-main/README.md) |
| 选择性解压的重名策略、父目录冲突与重试 | PASS 安装版 11 个真实窗口用例，保留选定范围并核对输出文件。[用例与结果](verification/round6/installed-main/selected-extraction.json) |
| 384 MiB 选定成员，任务运行时取消 | PASS 任务终态已取消，原有选中及未选目标的清单和哈希保持一致。[终态及文件](verification/round6/installed-main/selected-cancel.json) |

## 普通批量解压

| Windows 11 x64 实际操作 | 结果 |
| --- | --- |
| 三个 ZIP，中间与已有文件冲突 | PASS。候选显示两项成功、一项失败，可查明冲突原因。成功文件内容正确，原包、已有文件和哨兵的哈希均相同。[前后对照](verification/round5/ordinary-batch/README.md) |
| 在第二项停止，再点「继续未开始的项目」 | PASS。在相同会话中，第三项写入原计划的第三个目录。第一项文件的身份、时间和长度不变，第二项保留已取消状态。[窗口与文件回执](verification/round5/ordinary-batch/candidate/receipt.json) |
| 关闭并重开程序后继续普通批次 | NOT RUN。当前继续入口针对本次会话。 |
| 普通批量完整性检查，第二个路径缺失 | PASS。检查弹窗与批量页均显示两项成功、一项失败。展开可查缺失项，两个有效 ZIP 和旁文件均原样保全。[窗口及文件回执](verification/round5/ordinary-batch/candidate/integrity.json) |

## 嵌套编辑窗口、关闭与恢复

| 输入与操作 | 验收 |
| --- | --- |
| R3 正常关闭双层 ZIP | PASS。实际点窗口关闭并取消后工作副本仍在。选择放弃后，新进程显示原成员；选择保存并关闭后，新进程显示已改名成员。逐项核对原外包、其余成员和同目录文件。[真实窗口与进程退出方式](https://github.com/qing-yi-5427/qing-zip/pull/35) |
| 恢复候选 b6201de，包括 64 MiB 普通内包 | PASS。强退、冷启动、原生另存、再次冷启动。导出文件 SHA-256 与已改内包一致，两个原包及相邻成员经核对。加密外包和普通内包组合按预期不登记，原包可用原密码重新读取。[功能验收](https://github.com/qing-yi-5427/qing-zip/actions/runs/45146040536) · [大样本](https://github.com/qing-yi-5427/qing-zip/actions/runs/45146592216) |
| 两个同名工程的三层恢复 | PASS。按原路径区分 A 和 B，取消实际另存窗口后仍有两项。向后端提交无效目的路径也保留两项。通过实际另存窗口导出 B 后只剩 A，另存 A 后为空，逐项比较工作副本及外包。[操作收据与截图](https://github.com/qing-yi-5427/qing-zip/pull/35) |
| R3 编辑后强制结束应用 | FAIL（从恢复菜单找回）。临时内包与中断前编辑副本的 SHA-256 相同，冷启动两次恢复列表都是零项。[截图及各项哈希](https://github.com/qing-yi-5427/qing-zip/pull/35) |
| 三层 ZIP 写回时外包已被其他程序改动 | R3 错标最内层。候选通过，待保存标记移到中间层，中文错误只出现一次；外部修改过的原包和各层相邻文件保全，合成用户数据可独立校验。[候选 #36](https://github.com/qing-yi-5427/qing-zip/pull/36) |
| 清洁状态读出后有编辑刚好完成，再自动关标签 | R3 回归失败，候选通过。此时序由确定性前端测试调度；常规取消、保存和放弃另有真实窗口对照。[回归 #36](https://github.com/qing-yi-5427/qing-zip/pull/36) |

## 第三轮交付便携包

组合源码 `fa5ecd5` 在托管 Windows x64 与原生 ARM64 均完成构建及普通回归。两架构均核对实际下载包、ZIP 目录和全部 CRC。下列项目使用交付的同一份可执行文件。[包哈希、来源、测试与截图索引](verification/round3/README.md)

| 入口 | 状态与证据 |
| --- | --- |
| x64 包的直接 ZIP 更新 | PASS。三轮应用强制退出后写入增长为零，并行任务、原包和重试均通过。另在剩余 0 字节和 6 MiB 的测试盘上更新失败后原包保全，释放空间后重试成功。[中断](verification/round3/zip-interruption.jsonl) · [满盘](verification/round3/update-zip-space-receipt.json) |
| x64 包的嵌套写回 | PASS。真实窗口在原盘已满时另存到另一卷，释放测试盘空间后在原标签写回，工作副本、另存文件与实际写入内包 SHA-256 相等。[两条恢复路径](https://github.com/qing-yi-5427/qing-zip/tree/65b0298/docs/verification/nested-save-capacity/integrated) |
| x64 包常见格式 | qing-zip 10/10 PASS。另三款工具共 24 个适用组合有 22 次 PASS、2 次 FAIL，失败的是 7-Zip、Bandizip 处理同一组旧式 RAR 存储三卷，官方 UnRAR 与 qing-zip 的结果一致。ZIP/7z 双向互读另行通过。[逐项清单](verification/round3/format-comparison.jsonl) · [双向互读](verification/round3/interop.json) |
| ARM64 包的真实菜单 | PASS。Windows 11 26200、实际 100% 缩放上切换普通、高对比度、减少动画并恢复。四种状态均显示 7 项。[配对截图与恢复回执](https://github.com/qing-yi-5427/qing-zip/pull/31) |

**BLOCKED（真实 200% 缩放）。** Windows 11 ARM64 托管会话切换三种系统显示分辨率，1024 × 768、1600 × 1200 和 1920 × 1080 均只提供 100% 和 125%。原来的显示模式、Scale 和测试设置已恢复，应用窗口前后两张截图字节相同。[系统选项与实际窗口](https://github.com/qing-yi-5427/qing-zip/pull/32)

## 独立安装候选与 Explorer 验收

| 实际操作 | 结果 |
| --- | --- |
| 从 Windows 11 ARM64 一级菜单解压到此处、智能解压和创建普通 ZIP | PASS。提取位置、成员和源哈希通过，退出窗口后再读新 ZIP 通过。[三项操作](https://github.com/qing-yi-5427/qing-zip/pull/34) |
| 移动已注册的便携目录、恢复菜单、单独注销 | PASS。仅移动时分组消失，旧位置注销并在新位置重新注册后实际解压成功。注销后分组消失，731 个文件哈希一致。[移动生命周期](https://github.com/qing-yi-5427/qing-zip/pull/33) |
| 官方 v0.12.20 升级至独立 x64 安装候选 | PASS。无交互及取消时原样保留，选中的两项成功移入恢复目录，另两项因被占用或已更改而保留，重复升级和卸载后恢复通过。[安装候选](https://github.com/qing-yi-5427/qing-zip/pull/27) |

## 历史基线与各主题验证

| 层级与入口 | 样本和断言 | 状态 |
| --- | --- | --- |
| 前端语法和 Node 回归 | 16 个 JS 语法检查，42 个测试文件，271 项通过。覆盖归档列表、键盘焦点、压缩参数、深度解压交互、密码本、任务重试、校验结果和右键转交队列。 | PASS |
| Python 运行时和发布脚本 | 校验 65 个 x64 运行时文件及 NanaZip 头文件。附加运行时清单、ARM64 包形状、修改 crate 源码来源和 ARM64 GUI 生命周期测试，共 8 项。合成普通 `.lnk` 文件的锁、内容更改和失败清理共 4 项。 | PASS |
| Rust 核心单元 | 32 项。列表 JSON 兼容、ZIP 时间字段、父目录任务顺序、运行时路径、LZMA 参数和报告，以及深度解压的计划、字节预算、分卷和发布状态。空间不足使用内存写入回调。 | PASS |
| Rust 深度解压集成 | 15 项经审阅的普通归档用例。多入口、错扩展名和无扩展名、ZIP/7z/RAR5、压缩 TAR、嵌套目录、分卷、选取、AES 密码续接、冲突重试和保留普通数字伴随文件。回收资格只读取会话报告，样本保留在临时目录。 | PASS |
| Rust CLI 集成 | 解压和测试的线程参数 1 项。校验工具的目录递归、输入顺序、不可读输入和打开写句柄共 3 项。 | PASS |
| Rust 引擎取消集成 | 取消后开始列包应返回取消。压缩普通 16 MiB 文件为 7z 的过程中触发取消，断言取消返回且目标包未发布。 | PASS（2 项） |
| 日常归档回归 | ZIP/7z 加密与错误密码；三种压缩 TAR 的内容和修改时间；GBK 代码页；更新失败保全及两次并发添加；两层嵌套、同名兄弟、外层变更、加密写回；ZIP 与固实 7z 各自明文和加密状态的四种更新模式。 | PASS（10 项精确命名的 Rust 用例，包含 16 种更新组合） |
| 真实文件占用与重试 | 普通 ZIP、7z 解压到已有文件，测试以 Windows 独占句柄持有该文件。选择覆盖应返回可处理错误，现有文件、目录和原包保持完整，释放句柄后再提取 1 个文件且内容正确。 | PASS（本地基线包和组合候选包分别复测） |
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
| 前台桌面、DPI 和真实 Explorer | M3 跟踪前台应用与 DPI。旧式右键和真实拖出仍需独立环境。 | NOT RUN（本矩阵） |
| 托管 Windows x64 NSIS 安装、升级及候选卸载 | 官方 v0.12.20 全新安装至一次性 runner 临时目录，原位升级至 0.12.23，核对登记与候选文件哈希，校验并解压旧 ZIP。卸载时移除候选发行文件、登记和开始菜单快捷方式。[首次验收](https://github.com/qing-yi-5427/qing-zip/actions/runs/45112224372) | PASS（所列阶段） |
| 托管 x64 卸载前后合成用户数据 | 独立候选补测的来源、原 ZIP、解压结果和安装目录内外两个哨兵，五项前后 SHA-256 相等。[补测收据](https://github.com/qing-yi-5427/qing-zip/actions/runs/45113902845) | PASS（五项） |
| 托管 x64 卸载后的旧版残留 | 卸载后仍有十个旧版独有文件，与初装和正式发行文件的哈希一致。候选发行文件全部移除。当前提案保守保留无逐文件归属记录的历史文件。[首次验收](https://github.com/qing-yi-5427/qing-zip/actions/runs/45112224372) · [补测结果](https://github.com/qing-yi-5427/qing-zip/actions/runs/45113902845) · [清单及迁移提案](https://github.com/qing-yi-5427/qing-zip/pull/21) | FAIL（严格残留断言） |
| 默认安装目录、ARM64 安装和文件关联 | 普通 NSIS 用例仅覆盖托管 x64 的临时路径。默认路径、ARM64 安装、升级及卸载、真实用户数据、安装所设文件关联及升级后保留未验收。 | NOT RUN |
| Windows 11 ARM64 菜单及系统无障碍 | 系统实际 100% 缩放、运行中切换高对比度和减少动画，7 项菜单一直可见，纯色/渐变及媒体查询符合系统，最后恢复系统原值。[四次截图与回执](https://github.com/qing-yi-5427/qing-zip/pull/13) | PASS |
| Windows 11 现代右键 | 一次性 Windows 11 ARM64 以临时签名注册，真实点击「解压到此处」与「智能解压」，分别在同级和同名目录得到两个预期成员。普通文件的压缩入口保存了 ZIP，独立读取唯一成员与来源 SHA-256 相同。原包、来源和哨兵保全。[动作与截图](https://github.com/qing-yi-5427/qing-zip/actions/runs/45133376060) | PASS（三项操作）。正式发行签名及升级集成待验收。 |
| Windows 11 ARM64 便携菜单生命周期 | 仅移动外置目录时注册仍指旧路径，一级分组消失。按顺序注销、移动、重注册后，实际点击从新路径解压。最终保留应用开关仅注销包，新 Explorer 窗口无 qing-zip 分组，注册和测试证书清理通过。[WinRT 路径和实际菜单](https://github.com/qing-yi-5427/qing-zip/actions/runs/45132964898) | PASS（临时测试签名包） |
| Windows 10 与 Windows 11 ARM64 | Windows 11 ARM64 托管菜单及组件已见上方实测。Windows 10、实体 ARM64 设备及正式安装后 Explorer 的体验仍待验证。 | BLOCKED（所列未覆盖环境） |
| 普通 ZIP 目标卷已满 | 托管 Windows x64 的独占 NTFS 卷可用空间为零。创建与覆盖解压报告空间不足。已有目标、无关文件、来源和原包均保全，释放空间后重试成功。[逐次回执](https://github.com/qing-yi-5427/qing-zip/pull/22) | PASS（普通 ZIP 两种操作） |
| 覆盖解压写入期间耗尽空间 | 普通 32 MiB 存储 ZIP 的暂存文件实际增长至 29,327,360 字节，观测内容前缀与输入一致。当时还需写 4,227,072 字节，卷上仅余 24,576 字节。原生错误后原包、已有目标、无关文件与暂存目录清单保持，腾出空间后同命令成功，32 MiB 输出通过 SHA-256。[写入期间耗尽的复跑](https://github.com/qing-yi-5427/qing-zip/actions/runs/45121239407) | PASS（两台独立 runner） |
| 第二轮组合包的原生 ARM64 常用操作 | 在 Windows 11 ARM64 构建同一提交的便携包。经典 RAR 存储分卷的校验与提取、包内 ZIP 缓存和取消实跑通过。安装包完成构建，安装生命周期和该组合包的真实 ARM64 窗口未在此运行中执行。[原生 ARM64 验收](https://github.com/qing-yi-5427/qing-zip/actions/runs/45119972417) | PASS（所列操作） |
| 已有 ZIP 更新时空间不足 | 目标卷开始时剩 0 B 或 6 MiB。实际 CLI 同时加入 16 MiB 文件并替换一个已有成员，失败后原包整体、原成员、输入与邻近文件均保全。腾出空间后同命令成功，独立读取器核对新建、保留、替换的三个成员。[实测](https://github.com/qing-yi-5427/qing-zip/pull/26) | PASS（两组） |
| 真实窗口嵌套 ZIP 满盘写回 | 打开内层 ZIP 并重命名成员。外层目标卷已满时点击保存，原包及邻近文件保持，编辑副本完整。释放空间后再次点击保存，写入的内包与编辑副本字节一致，兄弟 ZIP 和外层无关成员未变。[实机截图与复跑](https://github.com/qing-yi-5427/qing-zip/pull/30) | PASS（同会话） |
| 普通 ZIP 更新时父进程异常退出 | 候选三轮及额外三轮外层 Windows Job 均在父进程退出采样点后零写入，原包保全，独立任务完成，重试通过。2–3 MiB 部分候选仍保留。[逐轮原始记录](https://github.com/qing-yi-5427/qing-zip/pull/28) | PASS（限定直接 ZIP 候选写入阶段） |
| 满盘后的可访问提示 | 原版同一异常出现两次，候选只出现一个可访问提示并显示真实归档路径。关闭弹窗后在同一标签重试成功。前端共 278 项通过。[配对截图和数据](https://github.com/qing-yi-5427/qing-zip/tree/af2045c73c10f402a8239d1b34399658f8e21a9f/docs/verification/nested-save-capacity) | PASS（Windows x64） |
| 特殊文件系统 | ADS/TxF、重解析及链接另在获准的隔离环境验证。 | BLOCKED（未启用相应环境） |
| 扩展格式、嵌套编辑及其他损坏模式 | 尚待扩大独立对照。普通嵌套写回已见上方日常回归。嵌套编辑重启恢复、ZIPX、传统加密 ZIP、ALZ/EGG、ISO、其他损坏类型与 ZIP 修复待扩大独立对照。 | NOT RUN |
| GUI 退出和重启后的恢复 | 引擎取消、密码续接和发布重试已有回归。选取解压取消验证了原有文件保全及句柄释放；真实 CLI 父进程中断、重开和重试另列于下。普通 TAR.GZ 的真实窗口取消与既有目标保全已通过；GUI 退出及重启恢复仍待实测。 | NOT RUN（上述端到端场景） |
| 异常路径和数量预算 | 深度、数量和字节计费由内存用例验证，也执行了正常样本的选取预算。异常路径及大量嵌套、超文件数的真实归档场景尚待验证。 | NOT RUN（实包边界） |
| 真实任务取消和结果入口 | 普通 1 GiB TAR.GZ 进入运行态后点击取消，保存中间及终态显示，原包和两个既有目标内容不变。另一次完成后点开输出按钮，核对真实 Explorer 窗口位置。[实机记录](https://github.com/qing-yi-5427/qing-zip/pull/6) | PASS |
| 真实回收与还原 | Windows 托管 C:、D: 卷分别核对选取部分文件、跳过冲突、提取失败、完整解压。前三项保留原包，最后一项通过系统回收站按原路径还原，源包 SHA-256 一致。[四种结果与同父目录对照](https://github.com/qing-yi-5427/qing-zip/pull/17) | PASS（普通 ZIP） |
| ZIP/7z 双任务长跑 | Windows x64 连续 1,800.062 秒，1,912 轮观测到进程重叠，3,824 项任务均完成。逐轮复核输入、输出哈希与工作目录清理。[托管运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45108701387) | PASS（半小时普通合成样本） |
| 普通归档更新中断 | 三轮强制结束正在更新普通 ZIP 的 CLI，重新打开、校验原包及输入哈希，独立任务继续、重试成功。每轮遗留 134,218,001 字节未发布候选。[逐轮记录](https://github.com/qing-yi-5427/qing-zip/pull/10) | PASS（原包保全及重试） / FAIL（候选回收） |
| 智能解压桌面入口 | 标准、深度编号、批量、首页文件选择器、已有实例接收命令五条路径核对实际输出。另从真实 Explorer 将错扩展名 ZIP 拖入首页，核对原生悬停提示、识别预告、实际输出和原包哈希。[实机记录](https://github.com/qing-yi-5427/qing-zip/pull/9) | PASS（五种入口与原生拖入） |
| 经典 RAR 独立工具对照 | 存储三卷、压缩单卷和旧式两卷均与官方 UnRAR 内容一致。三卷首版误报 CRC 的证据单独保留。[旧版结果](https://github.com/qing-yi-5427/qing-zip/pull/7) · [修复与回归](https://github.com/qing-yi-5427/qing-zip/pull/25) | PASS（已测样本） |
| 加密 ZIP 任务失败后重试 | 在真实 Tauri 窗口观察中文诊断。失败时核对原包、输入与已有目标，正确密码重试并复核最终输出哈希。[窗口和哈希](https://github.com/qing-yi-5427/qing-zip/pull/23) | PASS |
| Rust 格式检查 | `obz-rs` 与 `obz-gui` 均有既有 rustfmt 差异，例如 `obz-rs/build.rs` 和 `obz-gui/src/archive_listing.rs`。 | FAIL |

产物 `qing-zip-0.12.23-portable.zip` 为 33,340,323 字节，SHA-256 为 `61918ca1723598776e79de5ed0f677336ccc2b5c22e5951d49f291dd48e8ab09`。四类普通归档与引擎基准见 [性能基线](PERFORMANCE_REPORT.md)。

常用格式对照的细节见 [质量报告](QUALITY_REPORT.md)。Shell 包和原生接口的证据见 [M4 验证记录](https://github.com/qing-yi-5427/qing-zip/pull/1) 与 [原生 x64 / ARM64 托管运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45096359747)。

第一轮审查的组合源码为 `e9eeecd`，x64 便携 ZIP 共 33,286,633 字节，SHA-256 为 `7611a000a3f6ffcc4a93fdb3c6e8189b9b563fd95cfe88019530b59060d72932`。实际包内 worker 完成新加的选择性提取取消用例，观察到 95 次读操作。此候选运行 276 项 Node 回归，真实 ZIP/7z 冒烟及常用格式比对均已复跑。[可复建源码](https://github.com/qing-yi-5427/qing-zip/tree/e9eeecd414fa7379fc959967fb1645ced24eeb62) · [测试实现及控制组](https://github.com/qing-yi-5427/qing-zip/pull/15)

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

日常归档回归可在仓库根目录运行 `./scripts/check-daily-archive-regressions.ps1`。每个名称必须实际执行并通过 1 项测试。文件占用与解锁重试复用上面的 `check-public-package.py`。两项均进入常规 Windows x64 工作流。

## 公开 CI 历史记录

以下记录对应早一版公开提交 `517ef21a6dcac2cee9cd438ee3053597220e3436`，平台覆盖仍有参考价值。

| CI | 已通过 | 跳过 |
| --- | --- | --- |
| [Windows x64 regression · 45003050224](https://github.com/qing-yi-5427/qing-zip/actions/runs/45003050224) | 已发布运行时校验、有限前端/纯 Rust、深度解压正常样本、加固 x64 便携包。 | 完整前端、原生 Explorer、核心扩展套件、原生链接与桌面回归、注册后 Explorer 测试均为该运行的 `SKIP`。 |
| [Windows ARM64 native validation · 45003050222](https://github.com/qing-yi-5427/qing-zip/actions/runs/45003050222) | 原生运行时和工具、选定纯回归、ARM64 便携包和安装包构建、源码包。 | 原生 CLI/分卷、ADS/链接、经典菜单、安装及卸载的实跑为 `SKIP`。 |
| [Windows EFS regression · 45003050182](https://github.com/qing-yi-5427/qing-zip/actions/runs/45003050182) | 运行状态为 `SKIP`。 | EFS 真实操作。 |

## 静默升级沿用旧安装位置

Windows 11 ARM64 从官方 0.11.27、Windows Server 2025 x64 从官方 0.12.20 升级至 0.12.23，均为 PASS。初装使用独立临时目录，升级时安装器自动使用登记位置。逐件比对发行文件，实测解压旧版创建的 ZIP，卸载后五份合成用户数据 SHA-256 保持一致。[双架构的原始文件结果与复跑方法](verification/native-installer-upgrade/README.md)
