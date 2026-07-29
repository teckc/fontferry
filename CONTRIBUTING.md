# Contributing

开发基线为 Rust 1.97.1、Node.js 24 LTS 和 pnpm 11.9.0。Windows 使用 MSVC
工具链；macOS 需要 Xcode Command Line Tools；Linux 需要 Tauri 2 的 WebKitGTK
构建依赖。

提交前运行：

```text
cargo xtask check
cargo deny check
```

新增内置字体应修改 `catalog/builtin/catalog.json`，确认许可证、Release 资产正则及三平台
支持情况。目录分支发布物必须由目录专用 Ed25519 密钥签名，不能复用应用更新密钥。

Actions 依赖必须固定到完整 commit SHA。字体文件、商标、签名私钥、平台证书、Token、
数据库、日志及构建产物不得提交。
