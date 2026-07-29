# FontFerry 发布与目录签名

FontFerry 使用两套相互独立的 Ed25519 密钥：

- Tauri updater 密钥只签名程序更新制品。
- catalog 密钥只签名远程字体目录。

私钥不得提交到 Git、写入命令行参数或只保存在 GitHub Secrets。至少保留一份离线加密备份。

## 1. 程序更新密钥

在受信任的本机执行 Tauri 官方密钥生成命令：

```text
pnpm tauri signer generate --write-keys
```

将公钥写入 `apps/fontferry/src-tauri/tauri.conf.json` 的
`plugins.updater.pubkey`。将私钥和可选密码写入 GitHub Environment
`release`：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

不要在私钥尚未备份、配置尚未推送时创建 `v0.2.*` 标签。发布工作流拒绝没有 updater
私钥的构建。

## 2. 目录密钥

生成 32 字节随机私钥并只放入当前 PowerShell 进程：

```powershell
$env:FONTFERRY_CATALOG_SIGNING_KEY = & openssl rand -base64 32
cargo xtask catalog-public-key
```

把输出的公钥写入 `catalog/public-key.txt`。创建远程目录时：

```powershell
cargo xtask validate-catalog
cargo xtask sign-catalog catalog/builtin/catalog.json catalog.json.sig
```

将原始 JSON 字节与 `catalog.json.sig` 一起提交到同仓库的 `catalog` 分支。签名针对文件
的精确字节；格式化 JSON 后必须重新签名。发布目录前，应在干净工作树中验证应用能够
下载、验签并写入缓存。

## 3. 平台签名边界

- Windows alpha/beta 可以明确标注为未签名，但会触发 SmartScreen 警告；稳定版应配置代码签名证书。
- macOS 仓库外分发的稳定版应使用 Developer ID 签名并完成公证。
- updater Ed25519 签名不能替代 Windows Authenticode 或 macOS Developer ID。
- deb/rpm 由 apt/dnf 更新，应用内 updater 只处理 AppImage。

## 4. 发布检查

1. `cargo xtask check`
2. 三平台 CI 与 dependency policy 全部通过。
3. Windows、macOS、Ubuntu/Fedora 实机完成安装、更新、卸载和字体回滚。
4. 创建版本标签；Release 工作流生成 SHA-256、CycloneDX SBOM 和 provenance attestation。
5. 用旧版本实际执行一次 updater 恢复演练。
