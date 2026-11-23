# NSIS 旧发行版残留处理提案

官方 x64 v0.12.20 原位升级至 0.12.23 后，卸载只处理 0.12.23 的发行文件。一次性 Windows x64 运行确认，两项旧 ZPAQ 可执行文件和八项旧许可证或说明文件仍在，共 2,287,129 字节。当前 ZPAQ 读写调用 `runtime/zpaq/obz-zpaq.exe`，安装残留中的两个旧文件名未出现在该调用路径。[NSIS 卸载清单](../../scripts/installer.nsi) · [ZPAQ 调用](../../obz-rs/src/rare.rs)

## 托管实测

[PR #19](https://github.com/qing-yi-5427/qing-zip/pull/19) 的[托管运行 45112224372](https://github.com/qing-yi-5427/qing-zip/actions/runs/45112224372) 使用 PR head `e8597900eaa6fea422120c2ea84b8524e692cfb7` 和检出提交 `60cf559d0273ef5ea0070f165a8bece32e112dbf`。旧版初装、原位升级、旧归档校验与解压、候选 payload 及产品登记和快捷方式卸载的各阶段均通过。最终“安装目录仅余合成用户文件”断言失败。收据记录十个与旧版初装及正式发行包逐项一致的额外文件、两个预期用户文件，以及零个残留的候选发行文件。此次断言失败发生在最终的用户数据哈希复验前。[收据对应的验证脚本](https://github.com/qing-yi-5427/qing-zip/blob/e8597900eaa6fea422120c2ea84b8524e692cfb7/scripts/check-nsis-upgrade-hosted.py)

- 官方 v0.12.20 x64 安装器 SHA-256 `83a08671edde7bb53ff37582f4d82fd880c968029ddcd39001119efc11a979e1`。
- 官方 v0.12.20 x64 便携包 SHA-256 `6acef9b5a4ae726b6e6bd9bad4eecda5b4238d5f60296cc4b5d74e7671efb84a`。
- 该次托管构建的候选安装器 SHA-256 `70c57e052ed6a7d0df8095279b1c3c68e935794dab07bc573ad9e57004102b29`。

下表只涵盖该 x64 样本。另行下载官方便携包并核对完整 ZIP SHA-256 后，读取十个 ZIP 成员，逐项复算了文件长度和 SHA-256，并与同一次托管收据中的旧版初装及卸载残余两份记录核对。[官方 v0.12.20 发行页](https://github.com/qing-yi-5427/qing-zip/releases/tag/v0.12.20)

| 安装目录相对路径 | 字节数 | 正式发行文件 SHA-256 |
| --- | ---: | --- |
| `licenses/7zip-26.05/License.txt` | 5,885 | `32369594a3a9f7c643d124035120eaa6a7707e75e57c4386ef509f801447bc49` |
| `licenses/7zip-26.05/README.txt` | 433 | `8aec3ac469c6aae7adc8424e234a256c6d885c9c173abdc0312eb7ec20d46ab7` |
| `licenses/python-3.14.9/LICENSE-ACEFILE.md` | 2,117 | `b78ee8da84166064863c3f8f0ffce4768393c56ba6e5c68412d2198fb5ebcdb3` |
| `licenses/python-3.14.9/LICENSE.txt` | 34,824 | `939d8e9fa591cbaff0d51e777edb4c1002045d2600ce43a1ae28530cecfc5b88` |
| `licenses/rust/form_urlencoded-1.2.2/LICENSE-APACHE` | 10,847 | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `licenses/rust/form_urlencoded-1.2.2/LICENSE-MIT` | 1,072 | `20c7855c364d57ea4c97889a5e8d98470a9952dade37bd9248b9a54431670e5e` |
| `licenses/rust/url-2.5.8/LICENSE-APACHE` | 10,847 | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `licenses/rust/url-2.5.8/LICENSE-MIT` | 1,072 | `b38f11f6096706e6de553dabe2a7ed142d59b6fa8c97e290c67496154745cdd5` |
| `runtime/zpaq/zpaq.exe` | 1,094,656 | `a0f127a70943b0262060498c1723c795a8e2980f1acf0c42ee8c1dae72ae54b5` |
| `runtime/zpaq/zpaq64.exe` | 1,125,376 | `7a94b4f1d6323a758c7b0b6344036f166bff0fd44f1c3c86f05b3688023496cb` |

## 自动清理边界

v0.12.20 的 HKCU 产品登记记录安装路径、版本及卸载命令。发行清单和 SHA-256 可以辨认主数据与发行文件一致的候选项。即使安装路径、版本、相对路径和散列全部一致，用户也可能后来放入内容相同的文件。旧安装器没有记录每项安装时的文件对象身份，升级时无法区分两种来源。曾升级过的目录还可能包含多个历史版本的文件。扩充本版的卸载名单或运行旧版卸载器会把这些未经验证的情形纳入删除范围。[v0.12.20 安装脚本](https://github.com/qing-yi-5427/qing-zip/blob/v0.12.20/scripts/installer.nsi)

现有快捷方式 helper 先读取安装时的独立归属记录，再通过同一个文件句柄验证并标记删除。它的记录在创建时包含安装路径、文件身份和内容。引擎还提供保持祖先目录名称固定并拒绝重解析点的句柄验证。按句柄操作解决检查后被替换的问题，无法补回旧版缺失的归属记录。Windows 还按文件流处理共享和机会锁。主数据 SHA-256 不能证明整项文件没有用户内容，枚举后直接永久删除也不能排除这段时间有其他流被写入。[快捷方式 helper](../../obz-gui/src/associations/shortcuts.rs) · [祖先目录固定](../../obz-rs/src/engine_stream.rs) · [Microsoft 文件流说明](https://learn.microsoft.com/en-us/windows/win32/fileio/file-streams) · [Microsoft 句柄操作说明](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)

## 最小可行方案

1. **现有 v0.12.20 安装保留残留。** 如需在产品中显示迁移提示，只列在已登记的旧 x64 安装根中匹配固定名单、长度和哈希的十个候选项。NSIS 已有 HKCU 路径及版本读取能力。读取时检查产品安装目录、卸载登记路径与选定目标是同一个普通目录，版本匹配 `0.12.20`，并固定目录和文件句柄。仅处理普通单硬链接文件，遇到重解析点或无法确定的属性就保留。缺登记、路径不同、无法验证、已修改及未知同名均保持原位置。显示候选时说明内容一致的含义，由用户逐项选择要处理的文件。实际处理前再次核对对象身份和内容。已经升级并失去旧版登记的目录只保留。
2. **将来可先加归属记录，再做回收。** 先选定小范围、会随版本退役的安装器专属文件。沿用 NSIS 到已安装 helper 的调用方式，在 helper 确认本次确实创建文件时写入版本化归属记录，包含产品架构、安装目录身份、相对路径、文件身份和发行 SHA-256。由创建操作直接提供同一对象的身份，禁止事后扫描安装目录补认领。验证失败或写入收据失败时，后续版本保留对应项。在下一版本，仅对有旧的有效收据、目录和文件身份相同、精确内容匹配、其文件确已退役的项目考虑迁移。
3. **需要可恢复处理时，优先同卷移动整个文件对象。** 使用已经验证的句柄和固定的目标父目录，原子重命名到卸载清单之外、用户可访问的恢复目录。目标存在即跳过。保留用户可见的原路径与恢复位置。从文件系统得到明确结果后才更新收据。用户未明确选中无收据的候选时保留原路径。失败时报告结果。若操作结果不明，先核对原路径与恢复目标，避免重试或覆盖。后续恢复遇到占位也应保留两份。微软建议采用 TxF 的替代方式，且安装器不能假定所有支持的卷都提供文件事务。[Microsoft TxF 建议](https://learn.microsoft.com/en-us/windows/win32/fileio/deprecation-of-txf)

提案不改变现有签名或注册流程。上述迁移和归属功能尚未实现，也未将其结果记为通过。

## 后续最低验收

- 复用 [PR #19 一次性 x64 验收](https://github.com/qing-yi-5427/qing-zip/blob/e8597900eaa6fea422120c2ea84b8524e692cfb7/docs/verification/nsis-upgrade-acceptance.md) 的固定资产、隔离路径和阶段收据。对旧版没有文件归属记录的用例，完整跑官方初装、候选原位升级、使用原归档、卸载。核对十项原版文件以及人为修改的退役文件有各自独立场景，文件名和哈希逐项与预期一致。当前残留断言仍记录为已观察到的差异。
- 对有收据后允许迁移的用例，证明两个普通发行版本之间只有符合收据的退役对象离开原位置，且恢复目录的文件和路径正确。用已修改的旧文件、相同字节但重新创建的文件、其他目录的同名文件、无登记或登记路径不匹配、归档和目录内外哨兵验证保留规则。成功与各项保留都需记录独立检查。正常安装、输出校验、更新注册、卸载及快捷方式回归仍须通过。
- 复验重复运行、恢复目标被占用以及操作意外中断后的保留规则。特殊属性和文件流验收只在另有批准的隔离环境中执行。本轮只读取正式 ZIP 和既有托管收据，未在本机运行安装、卸载或特殊文件系统测试。
