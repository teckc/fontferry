# Contributing

开发基线为 Rust 1.97.1、Node.js 24 LTS 和 pnpm 11.9.0。Windows 使用 MSVC
工具链；macOS 需要 Xcode Command Line Tools；Linux 需要 Tauri 2 的 WebKitGTK
构建依赖。

提交前运行：

```text
cargo xtask check
cargo deny check
```

内置字体的维护源是 `catalog/source.json`。确认许可证、发布附件匹配规则及平台支持后，
运行 `node scripts/generate-catalog.mjs` 更新内置目录，再运行 `cargo xtask validate-catalog`。
模板只允许声明现有规格，生成结果保留规格 ID，CI 检查生成结果与提交一致。
目录分支发布物使用目录专用 Ed25519 密钥签名。包含包内文件选择规则的目录使用 schema 2。


Actions 依赖必须固定到完整 commit SHA。字体文件、商标、签名私钥、平台证书、Token、
数据库、日志及构建产物不得提交。
