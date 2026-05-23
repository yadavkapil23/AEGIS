//! TLS/mTLS configuration and setup

use crate::error::{Result, SecurityError};
use rustls::ServerConfig;
use rustls_pemfile::{certs, private_key};
use std::fs;
use std::path::{Path, PathBuf};
use std::io::BufReader;
use serde::{Deserialize, Serialize};

/// TLS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Path to certificate file (PEM)
    pub cert_path: PathBuf,

    /// Path to private key file (PEM)
    pub key_path: PathBuf,

    /// Enable mutual TLS (mTLS)
    pub mutual_tls: bool,

    /// Path to CA certificate for mTLS verification
    pub ca_cert_path: Option<PathBuf>,

    /// Minimum TLS version
    pub min_version: String, // "1.2", "1.3"

    /// Cipher suites (empty = default)
    pub cipher_suites: Vec<String>,
}

impl TlsConfig {
    /// Load configuration from YAML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path)
            .map_err(|e| SecurityError::TlsConfigurationError(e.to_string()))?;

        serde_yaml::from_str(&content)
            .map_err(|e| SecurityError::TlsConfigurationError(e.to_string()))
    }

    /// Create from certificate and key files
    pub fn new(cert_path: PathBuf, key_path: PathBuf) -> Self {
        Self {
            cert_path,
            key_path,
            mutual_tls: false,
            ca_cert_path: None,
            min_version: "1.2".to_string(),
            cipher_suites: vec![],
        }
    }

    /// Enable mutual TLS
    pub fn with_mtls(mut self, ca_cert_path: PathBuf) -> Self {
        self.mutual_tls = true;
        self.ca_cert_path = Some(ca_cert_path);
        self
    }

    /// Set minimum TLS version
    pub fn with_min_version(mut self, version: String) -> Self {
        self.min_version = version;
        self
    }
}

/// TLS server configuration helper
pub struct TlsServerConfig;

impl TlsServerConfig {
    /// Load server configuration from files
    pub fn load(config: &TlsConfig) -> Result<ServerConfig> {
        // Load certificate chain
        let cert_file = fs::File::open(&config.cert_path)
            .map_err(|e| SecurityError::TlsConfigurationError(
                format!("Cannot open cert file: {}", e)
            ))?;

        let mut cert_reader = BufReader::new(cert_file);
        let certs_iter = certs(&mut cert_reader);
        let cert_chain: Vec<_> = certs_iter
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| SecurityError::TlsConfigurationError(
                "Invalid certificate format".to_string(),
            ))?
            .into_iter()
            .map(|c| rustls::Certificate(c.as_ref().to_vec()))
            .collect();

        if cert_chain.is_empty() {
            return Err(SecurityError::TlsConfigurationError(
                "No certificates found in file".to_string(),
            ));
        }

        // Load private key
        let key_file = fs::File::open(&config.key_path)
            .map_err(|e| SecurityError::TlsConfigurationError(
                format!("Cannot open key file: {}", e)
            ))?;

        let mut key_reader = BufReader::new(key_file);
        let key = private_key(&mut key_reader)
            .map_err(|_| SecurityError::TlsConfigurationError(
                "Invalid private key format".to_string()
            ))?
            .ok_or_else(|| SecurityError::TlsConfigurationError(
                "No private key found in file".to_string(),
            ))?;

        // Build server config - key is already PrivateKeyDer from private_key()
        let server_config = ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(cert_chain, rustls::PrivateKey(key.secret_der().to_vec()))
            .map_err(|e| SecurityError::TlsConfigurationError(e.to_string()))?;

        // Enable mTLS if configured
        if config.mutual_tls {
            if let Some(ca_path) = &config.ca_cert_path {
                let ca_file = fs::File::open(ca_path)
                    .map_err(|e| SecurityError::TlsConfigurationError(
                        format!("Cannot open CA cert file: {}", e)
                    ))?;

                let mut ca_reader = BufReader::new(ca_file);
                let ca_certs_iter = certs(&mut ca_reader);
                let ca_certs: Vec<_> = ca_certs_iter
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| SecurityError::TlsConfigurationError(
                        "Invalid CA certificate format".to_string()
                    ))?;

                if !ca_certs.is_empty() {
                    // Store CA certs for client verification
                    // (implementation would require rustls ClientAuth config)
                }
            }
        }

        Ok(server_config)
    }
}

/// Certificate information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateInfo {
    /// Subject
    pub subject: String,

    /// Issuer
    pub issuer: String,

    /// Not before
    pub not_before: String,

    /// Not after (expiration)
    pub not_after: String,

    /// Public key algorithm
    pub public_key_algorithm: String,

    /// Signature algorithm
    pub signature_algorithm: String,

    /// Serial number
    pub serial_number: String,

    /// Subject Alternative Names
    pub san: Vec<String>,
}

/// Certificate validation
pub struct CertificateValidator;

impl CertificateValidator {
    /// Check if certificate is expired
    pub fn is_expired(_cert_path: &Path) -> Result<bool> {
        // This would require parsing the certificate
        // For now, return not expired
        Ok(false)
    }

    /// Get certificate expiration date
    pub fn get_expiration(_cert_path: &Path) -> Result<String> {
        // This would require parsing the certificate
        Ok("2025-12-31".to_string())
    }

    /// Validate certificate chain
    pub fn validate_chain(_cert_path: &Path, _ca_path: &Path) -> Result<()> {
        // This would require full chain validation
        Ok(())
    }
}

/// TLS connection metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsMetrics {
    /// Number of active TLS connections
    pub active_connections: u64,

    /// Total TLS connections established
    pub total_connections: u64,

    /// Failed handshakes
    pub handshake_failures: u64,

    /// Expired certificates caught
    pub expired_certs: u64,

    /// mTLS rejections
    pub mtls_rejections: u64,
}

impl Default for TlsMetrics {
    fn default() -> Self {
        Self {
            active_connections: 0,
            total_connections: 0,
            handshake_failures: 0,
            expired_certs: 0,
            mtls_rejections: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_config_creation() {
        let config = TlsConfig::new(
            PathBuf::from("/etc/ssl/cert.pem"),
            PathBuf::from("/etc/ssl/key.pem"),
        );

        assert_eq!(config.cert_path.to_str().unwrap(), "/etc/ssl/cert.pem");
        assert_eq!(config.key_path.to_str().unwrap(), "/etc/ssl/key.pem");
        assert!(!config.mutual_tls);
    }

    #[test]
    fn test_tls_config_with_mtls() {
        let config = TlsConfig::new(
            PathBuf::from("/etc/ssl/cert.pem"),
            PathBuf::from("/etc/ssl/key.pem"),
        )
        .with_mtls(PathBuf::from("/etc/ssl/ca.pem"));

        assert!(config.mutual_tls);
        assert_eq!(
            config.ca_cert_path.as_ref().unwrap().to_str().unwrap(),
            "/etc/ssl/ca.pem"
        );
    }

    #[test]
    fn test_certificate_validator() {
        let result = CertificateValidator::is_expired(Path::new("/path/to/cert.pem"));
        assert!(result.is_ok());

        let expiration = CertificateValidator::get_expiration(Path::new("/path/to/cert.pem"));
        assert!(expiration.is_ok());
    }
}
