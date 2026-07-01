//! TLS for remote access: user-supplied certs, or a persisted self-signed cert
//! generated on first run (TOFU — the browser warns once, then it's stable).

use std::path::PathBuf;

use axum_server::tls_rustls::RustlsConfig;

#[derive(Debug, Clone)]
pub enum TlsSource {
    Files { cert: PathBuf, key: PathBuf },
    SelfSigned { dir: PathBuf },
}

fn hostname() -> Option<String> {
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(windows))]
    {
        std::fs::read_to_string("/etc/hostname")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

pub async fn rustls_config(src: &TlsSource) -> anyhow::Result<RustlsConfig> {
    // rustls 0.23 needs an explicit default crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();

    match src {
        TlsSource::Files { cert, key } => Ok(RustlsConfig::from_pem_file(cert, key).await?),
        TlsSource::SelfSigned { dir } => {
            let cert_path = dir.join("cert.pem");
            let key_path = dir.join("key.pem");
            if !cert_path.exists() || !key_path.exists() {
                std::fs::create_dir_all(dir)?;
                let mut sans = vec!["localhost".to_string()];
                if let Some(h) = hostname() {
                    sans.push(h);
                }
                let certified = rcgen::generate_simple_self_signed(sans)?;
                std::fs::write(&cert_path, certified.cert.pem())?;
                std::fs::write(&key_path, certified.key_pair.serialize_pem())?;
                eprintln!(
                    "[conduit] generated self-signed TLS cert at {} (browsers will warn once)",
                    cert_path.display()
                );
            }
            Ok(RustlsConfig::from_pem_file(&cert_path, &key_path).await?)
        }
    }
}
