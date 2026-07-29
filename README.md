# FontFerry

FontFerry 是一个面向 Windows、macOS 和 Linux 的每用户字体更新管理器。它使用
Tauri 2、Svelte 5 与 Rust 构建，GUI 和无界面计划任务共用同一个安装引擎。

当前版本：`0.2.0-alpha.1`。

## 能力

- 从 GitHub Release、JSON 元数据、HTTP 指纹和 Font Awesome 官方发布 API 检查版本。
- 按字体选择变体；通常只启用一个版本，并保留上一版本用于回滚。
- 公开制品可自动安装；商业字体或无公开渠道的字体仅提醒。
- 用户可以通过声明式向导添加来源，不允许执行任意代码。
- 只安装到用户字体目录，只卸载 FontFerry 明确拥有的文件。
- 远程目录与应用更新分别使用 Ed25519 签名。
- 无遥测，不开放 localhost API。

## 开发

```text
pnpm install --frozen-lockfile
pnpm check
pnpm test
pnpm build
cargo xtask check
```

运行 GUI：

```text
pnpm tauri:dev
```

运行无界面更新：

```text
fontferry update --eligible --headless
```

## 发布边界

Alpha/Beta 可以提供明确标注的未签名平台安装包。正式稳定版必须配置 Windows
代码签名以及 macOS Developer ID 签名和公证。Tauri updater 制品始终要求更新专用
Ed25519 签名；deb/rpm 只通知，由 apt/dnf 更新。
密钥生成、目录签名和发布门禁见 [发布与目录签名](docs/release-signing.md)。

项目代码采用 Apache-2.0。字体文件、字体名称和商标不属于本项目许可证，详见
各上游许可证与 [NOTICE](NOTICE)。
