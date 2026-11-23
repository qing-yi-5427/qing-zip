# Windows 安装版的原位升级与卸载

在两台一次性托管 Windows 上，用官方安装器将旧版装到独立临时目录。旧版创建普通 ZIP，随后用 0.12.23 候选静默升级。升级命令只传 `/S`，验证安装器自行找到登记的旧目录。在该位置逐项匹配便携载荷，用已安装的 CLI 校验和解压旧包，然后实际卸载。

| 运行环境 | 官方旧版 → 候选 | 安装后逐项匹配的发行文件 | 卸载后保留的旧版专有文件 | 证据 |
| --- | --- | ---: | ---: | --- |
| Windows 11 ARM64 | 0.11.27 → 0.12.23 | 735 | 8 | [文件回执](arm64.json) · [原生运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45221662643) |
| Windows Server 2025 x64 | 0.12.20 → 0.12.23 | 739 | 10 | [文件回执](x64.json) · [x64 运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45221677861) |

两个系统上，卸载均移除了候选发行文件、卸载登记和所属快捷方式。五份合成用户数据逐一比较 SHA-256，卸载前后全部一致。保留的旧版文件已在初装后逐个对照该版本官方便携包。静默升级沿用现有保留策略。初次安装使用临时自定义目录，首次采用默认安装目录、真实签名的 Explorer 扩展升级仍需独立验证。

ARM64 可在 `Windows ARM64 native validation` 手动选择 `run_installer_upgrade` 复跑该流程。该步骤自行核对官方旧版资产 SHA-256。[托管验收脚本](../../../scripts/check-nsis-upgrade-hosted.py) · [ARM64 工作流](../../../.github/workflows/windows-arm64.yml)
