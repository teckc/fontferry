use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

#[derive(Debug, Parser)]
#[command(about = "FontFerry repository and release tasks")]
struct Xtask {
    #[command(subcommand)]
    command: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    Check,
    ValidateCatalog,
    CatalogPublicKey,
    SignCatalog {
        #[arg(default_value = "catalog/builtin/catalog.json")]
        input: PathBuf,
        #[arg(default_value = "catalog.json.sig")]
        output: PathBuf,
    },
    Checksums {
        directory: PathBuf,
    },
}

fn main() -> Result<()> {
    let task = Xtask::parse().command;
    match task {
        Task::Check => {
            run("cargo", &["fmt", "--all", "--", "--check"])?;
            run(
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
            )?;
            run("cargo", &["test", "--workspace"])?;
            run("pnpm", &["check"])?;
            run("pnpm", &["test"])?;
            run("pnpm", &["build"])
        }
        Task::ValidateCatalog => validate_catalog(),
        Task::CatalogPublicKey => {
            let signing_key = catalog_signing_key()?;
            println!(
                "{}",
                STANDARD.encode(signing_key.verifying_key().to_bytes())
            );
            Ok(())
        }
        Task::SignCatalog { input, output } => sign_catalog(&input, &output),
        Task::Checksums { directory } => checksums(&directory),
    }
}

fn validate_catalog() -> Result<()> {
    let body =
        fs::read_to_string("catalog/builtin/catalog.json").context("read built-in catalog")?;
    let value: serde_json::Value = serde_json::from_str(&body).context("parse catalog JSON")?;
    if value
        .pointer("/schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        bail!("catalog schemaVersion must be 1");
    }
    let fonts = value
        .pointer("/fonts")
        .and_then(serde_json::Value::as_array)
        .context("catalog fonts must be an array")?;
    if fonts.is_empty() {
        bail!("catalog must contain at least one font");
    }
    println!("catalog: {} entries", fonts.len());
    Ok(())
}

fn catalog_signing_key() -> Result<SigningKey> {
    let encoded = std::env::var("FONTFERRY_CATALOG_SIGNING_KEY")
        .context("FONTFERRY_CATALOG_SIGNING_KEY is not set")?;
    let bytes = STANDARD
        .decode(encoded.trim())
        .context("decode catalog signing key as base64")?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("catalog signing key must contain exactly 32 bytes"))?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn sign_catalog(input: &Path, output: &Path) -> Result<()> {
    let body = fs::read(input).with_context(|| format!("read {}", input.display()))?;
    let _: serde_json::Value = serde_json::from_slice(&body).context("parse catalog JSON")?;
    let signature = catalog_signing_key()?.sign(&body);
    fs::write(
        output,
        format!("{}\n", STANDARD.encode(signature.to_bytes())),
    )
    .with_context(|| format!("write {}", output.display()))?;
    println!("signed {} -> {}", input.display(), output.display());
    Ok(())
}

fn checksums(directory: &Path) -> Result<()> {
    let mut lines = Vec::new();
    for entry in WalkDir::new(directory).min_depth(1).max_depth(1) {
        let entry = entry?;
        if !entry.file_type().is_file() || entry.file_name() == "SHA256SUMS" {
            continue;
        }
        let mut file = fs::File::open(entry.path())?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        lines.push(format!(
            "{}  {}",
            hex_lower(&digest.finalize()),
            entry.file_name().to_string_lossy()
        ));
    }
    lines.sort();
    fs::write(
        directory.join("SHA256SUMS"),
        format!("{}\n", lines.join("\n")),
    )?;
    Ok(())
}

fn run(program: &str, arguments: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("run {program}"))?;
    if !status.success() {
        bail!("{program} failed with {status}");
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}
