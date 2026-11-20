# qing-zip 界面设计规则

主窗口和工具页加载 `obz-gui/frontend/ui.css`。共享变量和交互放在这里。窗口布局保留在 `desktop.css`，工具表单和结果布局放在 `tools.css`。

## 外观

新配置采用「跟随系统」。已有浅色或深色选择沿用。原生窗口先应用手动主题或解除覆盖，再读取 Windows 的有效主题。主文档、工具 iframe 和独立工具窗用同样的明暗设置。跟随系统时，原生主题变化触发更新。

Windows 11 可以使用 Acrylic 或 Mica。受支持的 Windows 10 版本可以使用 Acrylic，所选 Mica 回退为纯色。开启 Windows 高对比度时，窗口使用纯色，关闭后恢复材质偏好。

| 用途 | 变量 | 浅色 | 深色 |
| --- | --- | --- | --- |
| 窗口背景 | `--qa-base` | `#EDF2F9` | `#171F2C` |
| 工作区实色 / 半透明 | `--qa-surface` / `--qa-panel` | `#FBFCFE` / `#FBFCFEED` | `#212C3B` / `#212C3BEE` |
| 卡片和控件 | `--qa-card` | `#FFFFFF` | `#2A3546` |
| 次级层和选中背景 | `--qa-soft` / `--qa-tint` | `#F2F5FB` / `#EDF1FF` | `#273348` / `#303E60` |
| 正文 / 辅助和占位文字 | `--qa-text` / `--qa-sub` | `#26364C` / `#505F76` | `#EDF2FC` / `#A6B4CA` |
| 边框 / 分隔线 | `--qa-control-border` / `--qa-line` | `#8694A9` / `#E3E9F2` | `#8494AE` / `#354256` |
| 主按钮 / 悬停 / 按下 | `--qa-accent` / `--qa-accent-hover` / `--qa-accent-pressed` | `#3458CC` / `#2949B5` / `#243D98` | `#9DB6FF` / `#AFC3FF` / `#8BA8FA` |
| 主按钮文字 / 键盘焦点 | `--qa-on` / `--qa-focus` | `#FFFFFF` / `#3458CC` | `#182953` / `#C0D0FF` |
| 错误文字 | `--qa-danger` | `#A23040` | `#FFABB7` |

透明窗口的标题区使用 `--qa-glass`。辅助文字在浅色三个典型实色背景 `#EDF2F9`、`#FBFCFE` 和白色上达到 5.7∶1 以上。深色查找框的占位文字沿用其余控件的辅助色。

## 字体与间距

- 字体顺序为 Segoe UI Variable、Segoe UI、Microsoft YaHei UI 和系统无衬线字体。正文 14 px，控件 13 px，辅助说明 12 px。共用小标题和主标题分别为 21 和 25 px。
- 基础间距为 4、8、12、16、24 px。按钮图标和文字间距使用 8 px，左右内边距使用 12 px。圆角按输入框、按钮、工作区和弹窗分别用 8、9、16、18 px。
- 延用现有矢量图标。图标按钮用可访问名称说明动作，窗口最大化按钮的名称随还原状态更新。

## 控件状态

- 通用按钮用 `.qa-button`，主动作加 `.primary`，低强调动作加 `.quiet`，危险操作加 `.danger`。支持悬停、按下、键盘焦点、禁用和忙碌状态。已提交的工具动作通过禁用和 `aria-busy` 标记，任务状态仍由进度与文字呈现。
- 键盘焦点使用两像素轮廓。选择状态由控件语义和文字表达。高对比度模式使用 Windows 系统色，并为选中的主题、导航和表格项保留轮廓。
- 启用减少动画时，取消平滑滚动并缩短界面动画和过渡。进度内容、取消动作和最终结果照常更新。

## Windows 标题栏

Windows 11 使用按实际按钮位置更新的原生命中区，向系统报告最大化按钮。WebView 中的原按钮保留键盘焦点和最大化／还原动作。窗口最小宽度遵循工作区布局，窄贴靠区域能否使用取决于当前显示器和系统贴靠布局。 [Microsoft Snap 布局](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-snap-layout-menu)
