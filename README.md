# FontFerry（字渡）

FontFerry 是一个面向 Windows 和 macOS 的本地字体更新器。它从 GitHub
Releases 或直接下载地址发现版本，让你在浏览器界面中选择字体变体，
并按用户范围安装、更新、卸载和回滚字体。

## 特性

- 内置 Maple Mono、朱雀仿宋、霞鹜文楷、更纱黑体、得意黑、思源宋体、
  思源黑体和 MiSans 元数据。
- 一个字体可提供多个可勾选变体；同一字体只保留一个当前版本。
- 更新前保存上一版本，可执行一次回滚。
- 可从界面添加任意采用 GitHub Releases 的字体仓库。
- Windows 使用用户字体目录与 HKCU 注册表；macOS 使用
  `~/Library/Fonts`，均不要求管理员权限。
- 可启用每周自动更新：Windows Task Scheduler / macOS LaunchAgent。
- 下载支持断点续传、SHA-256 计算、2 GiB 上限和安全解压。
- 服务仅监听 `127.0.0.1`，写操作要求自定义请求头。

> FontFerry 不分发字体文件。每种字体仍受其上游许可证约束。MiSans
> 安装前必须在界面确认其许可证。

## 使用发行版

从 [Releases](https://github.com/teckc/fontferry/releases) 下载对应平台：

- Windows x64：解压后运行 `FontFerry.App.exe`
- Apple Silicon：解压后运行 `./FontFerry.App`
- Intel Mac：解压后运行 `./FontFerry.App`

macOS 首次运行未签名程序时，可能需要在“系统设置 → 隐私与安全性”
中确认打开。

程序启动后会在默认浏览器打开 `http://127.0.0.1:43717`。关闭终端窗口
即可停止本地服务。

## 从源码运行

需要 [.NET 10 SDK](https://dotnet.microsoft.com/download/dotnet/10.0)。

```powershell
dotnet run --project src/FontFerry.App
```

命令行更新所有已安装字体：

```powershell
dotnet run --project src/FontFerry.App -- update-all --no-browser
```

## 添加字体

在界面选择“添加 GitHub 仓库”，填写 `owner/repository`。FontFerry 会将
用户条目保存到应用数据目录，不修改程序安装目录。

如需把字体加入内置目录，向 `catalog/` 添加 JSON。格式与维护规则见
[CONTRIBUTING.md](CONTRIBUTING.md)。GitHub Release 条目的资源表达式应在
目标版本中恰好匹配一个文件。

应用数据目录：

- Windows：`%LOCALAPPDATA%\FontFerry`
- macOS：`~/Library/Application Support/FontFerry`

## 安全边界

FontFerry 会把下载内容视为不可信输入：限制下载大小、阻止压缩包路径穿越，
只提取 `.ttf`、`.otf`、`.ttc`、`.otc`，并在安装前解析字体元数据。
GitHub Actions 在 Windows、macOS、Linux 上构建和测试核心逻辑；实际字体
安装仅支持 Windows 与 macOS。

## 开源协议

FontFerry 源码采用 [Apache License 2.0](LICENSE)。
字体及其名称、商标、文件不属于该许可证的授权范围。
