# DevFlow Recorder

面向 Linux 开发者的本地工作流记录器。目标是自动整理应用、网页、窗口、项目与命令上下文，最后生成可复盘的中文时间线和日报。

## 当前 MVP

- Rust + Tauri 2 + React + Vite
- Wayland-first 采集架构
- Hyprland：通过 `hyprctl activewindow -j` 读取当前窗口
- Sway：通过 `swaymsg -t get_tree` 查找 focused 节点
- KDE/KWin：先做能力位展示，后续接 KWin 脚本/DBus 扩展
- GNOME/Mutter：普通 Tauri 应用无法直接读取全局活跃窗口；当前提供 GNOME Shell 42 扩展，通过 `127.0.0.1:45173` 上报焦点窗口
- 浏览器扩展桥：后端已预留 `ingest_browser_activity` 命令
- SQLite 本地持久化：启动时恢复当天时间线，活动与分段变化时写入本机数据库
- 终端上下文增强：只读取进程名、`argv0` basename 和 cwd，用于显示 `hermes x3`、`codex` 等轻量提示
- 默认隐私边界：不截图、不记录键盘、不读取终端命令参数，URL 移除 query 和 fragment

## 本地数据

运行数据默认保存在：

```text
~/.local/share/devflow-recorder/
```

其中：

- `devflow-recorder.sqlite` 保存当天活动与分段记录
- `bridge-token` 是 GNOME Shell 扩展调用本地 bridge 的临时 token

这些文件属于本机运行数据，不应提交到 Git。

## 开发命令

```bash
npm install
npm run build
npm run tauri:dev
```

## GNOME Shell 扩展

当前 Ubuntu GNOME Wayland 环境需要启用 Shell 扩展才能读取焦点窗口元数据。

```bash
./tools/install-gnome-extension.sh
gnome-extensions enable devflow-recorder@local
```

如果启用时报 “extension not found”，在 Wayland 下注销并重新登录后再执行 enable。启用后顶部栏会出现 `DF`，Tauri 后端会监听：

```text
http://127.0.0.1:45173/v1/gnome/window
```

扩展只上报窗口元数据：标题、应用名、app id、wm class、pid、workspace 和时间戳；不截图、不读取窗口内容、不记录键盘。

Tauri 后端启动时会生成本地 `bridge-token`。GNOME Shell 扩展会读取这个 token 并通过 `X-DevFlow-Token` 请求头上报，未携带 token 的本地 POST 会被拒绝。

## Wayland 采集原则

Wayland 默认不允许普通客户端全局读取其他窗口信息，所以采集必须按桌面环境分 provider：

- Hyprland/Sway 这类 compositor 提供命令接口，可以直接读取当前窗口元数据。
- KDE/KWin 需要通过 KWin 脚本或 DBus 做显式授权式扩展。
- GNOME/Mutter 对全局窗口元数据更保守，不能依赖 `qdbus` 直接读取当前窗口；应做 GNOME Shell 扩展，由扩展在用户授权后把窗口标题、应用 ID、PID 等元数据上报给本地服务。
- 浏览器 URL 不从系统层硬取，使用浏览器扩展主动上报当前激活标签页。

## 下一步

1. 实现浏览器扩展，通过本地桥上报标题和脱敏 URL。
2. 为 KDE/KWin 写 provider 探针。
3. 增加项目识别：Git repo、分支、VS Code/JetBrains 窗口规则。
4. 增加中文日报生成：先规则模板，后接 Ollama/OpenAI。
5. 增加设置页：采集开关、保留天数、导出/清空本地数据。
