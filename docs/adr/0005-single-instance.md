# 单实例强制（tauri-plugin-single-instance）

多开对启动器不是体验问题而是数据一致性风险：两个实例的看守轮询会对同一批 `~/.codex` 文件双写，taskboard 子进程会重复 spawn 抢端口。决定接入 tauri-plugin-single-instance，第二次启动不新建进程，转为激活已有实例并显示主窗口（复用现有 `show_main_window`，若最小化在托盘则恢复）。

**Consequences**：移除该保护会静默重新引入双写与端口冲突，且无任何报错提示——此插件是看守域正确性的前提之一，不是可选项。
