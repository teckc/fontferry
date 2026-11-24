# FontFerry 字渡

从原作者的发布源安装、更新和回滚字体。FontFerry 在当前用户范围管理自己安装的字体。商业字体或无公开下载的来源提供版本提醒和官方获取入口。

## 下载

目前提供 **v0.2.0-alpha.8 测试版**。

- [Windows x64 安装包](https://github.com/teckc/fontferry/releases/download/v0.2.0-alpha.8/FontFerry_0.2.0-alpha.8_x64-setup.exe)
- [macOS、Linux 下载与发行说明](https://github.com/teckc/fontferry/releases/tag/v0.2.0-alpha.8)

## 快速开始

1. 安装并打开 FontFerry，从「字体」页搜索一款字体，例如「得意黑」。
2. 打开字体详情，查看来源和许可，选择字体包，再点「安装字体」。核对显示的版本和文件后，点「确认安装」。
3. 安装完成后，在需要使用字体的应用里选择该字体。如果字体列表没有刷新，重新打开目标应用。
4. 已由 FontFerry 管理的字体可在详情里检查更新。存在上一版备份时，可恢复至该版本。

## 常见问题

- **连接 GitHub 失败。** 从字体详情重试或打开网络设置。在「GitHub 网络代理」可分别测试版本查询和字体下载。正常网络留空。已有本机代理时可填其 HTTP 或混合端口，保存后重启，返回字体详情重试。
- **应用内更新通道不可用。** 可从上面的发行页下载对应安装包。需要参与测试版更新时，可在「设置 → 程序更新」选择测试版通道。
- **移除已安装的字体。** 在字体详情点「卸载字渡管理的文件」。此操作只针对 FontFerry 管理的文件。移除应用前，可在设置中关闭每日自动更新。

## 产品与开发

[产品章程](docs/product.md) · [当前版本交付单](docs/milestone.md) · [架构与工程约束](docs/architecture.md) · [贡献指南](CONTRIBUTING.md)

FontFerry 使用 Tauri 2、Svelte 5 与 Rust。GUI 和无界面计划任务共用安装引擎。

```
pnpm install --frozen-lockfile
pnpm check
pnpm test
pnpm build
cargo xtask check
```

运行桌面界面。

```
pnpm tauri:dev
```

运行无界面更新。

```
fontferry update --eligible --headless
```

内置目录从 `catalog/source.json` 生成，维护命令见[贡献指南](CONTRIBUTING.md)。
目录签名、平台签名和更新包要求见 [发布与目录签名](docs/release-signing.md)。deb 和 rpm 由系统包管理器更新。

## 许可与隐私

项目代码采用 Apache-2.0。字体文件、字体名称和商标遵循各上游条款，见 [NOTICE](NOTICE)。应用无遥测。
