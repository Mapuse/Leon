//! `lbt` Secure Boot key management: generate a key set (PK/KEK/db), sign EFI
//! images with the db key, emit signature lists and verify signatures.
//! Thin wrapper over `openssl` / `sbsign` / `sbverify` / `cert-to-efi-sig-list`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::util;

/// Generate a PK/KEK/db key set under `--keydir`.
pub fn setup(keydir: Option<&str>) -> Result<()> {
    if !util::tool_available("openssl") {
        bail!("`openssl` not found; install it to generate keys");
    }
    let dir = keydir.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("keys"));
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let roles = ["PK", "KEK", "db"];
    for role in roles {
        let subj = format!("/CN=Leon {role}/");
        let key = dir.join(format!("{role}.key"));
        let crt = dir.join(format!("{role}.crt"));
        if key.is_file() && crt.is_file() {
            println!("{} already present, skipping", role);
            continue;
        }
        println!("generating {role} key pair");
        let status = std::process::Command::new("openssl")
            .args([
                "req",
                "-new",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-sha256",
                "-nodes",
                "-days",
                "3650",
                "-subj",
                &subj,
                "-keyout",
                key.to_str().expect("key path"),
                "-out",
                crt.to_str().expect("crt path"),
            ])
            .status()
            .with_context(|| format!("running openssl for {role}"))?;
        if !status.success() {
            bail!("openssl failed to generate the {role} key set");
        }
    }
    println!("key set written under {}", dir.display());
    println!("  PK:  {}/PK.key + PK.crt", dir.display());
    println!("  KEK: {}/KEK.key + KEK.crt", dir.display());
    println!("  db:  {}/db.key + db.crt", dir.display());
    Ok(())
}

/// Sign an EFI image with the db key.
pub fn sign(keydir: Option<&str>, path: Option<&str>, out: Option<&str>) -> Result<()> {
    let path = path.ok_or_else(|| anyhow::anyhow!("`keys sign` needs a `--path` to sign"))?;
    if !util::tool_available("sbsign") {
        bail!("`sbsign` (sbsigntools) not found; install it to sign EFI images");
    }
    let dir = keydir.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("keys"));
    let key = dir.join("db.key");
    let crt = dir.join("db.crt");
    if !key.is_file() || !crt.is_file() {
        bail!("missing db key set under {} (run `lbt keys setup`)", dir.display());
    }
    let out = out.map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{path}.signed")));
    util::run(
        "sbsign",
        &[
            "--key",
            key.to_str().expect("key"),
            "--cert",
            crt.to_str().expect("crt"),
            "--output",
            out.to_str().expect("out"),
            path,
        ],
    )?;
    println!("signed {} -> {}", path, out.display());
    Ok(())
}

/// Emit an EFI Signature List for a certificate (`.esl`).
pub fn esl(cert: &str) -> Result<()> {
    if !util::tool_available("cert-to-efi-sig-list") {
        bail!("`cert-to-efi-sig-list` (efitools) not found; install it to emit ESLs");
    }
    let out = format!("{cert}.esl");
    util::run(
        "cert-to-efi-sig-list",
        &[
            "-g",
            "8be4df61-93ca-11d2-aa0d-00e098032b8c",
            cert,
            out.as_str(),
        ],
    )?;
    println!("{out}");
    Ok(())
}

/// Verify an EFI image signature against a certificate.
pub fn verify(cert: &str, path: &str) -> Result<()> {
    if !util::tool_available("sbverify") {
        bail!("`sbverify` (sbsigntools) not found; install it to verify EFI images");
    }
    if !Path::new(path).is_file() {
        bail!("{} is not a file", path);
    }
    util::run("sbverify", &["--cert", cert, path])?;
    println!("signature OK: {}", path);
    Ok(())
}
