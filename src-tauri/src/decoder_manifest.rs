//! Versioned, signed decoder-plugin contract.
//!
//! The browser never supplies an executable or command line. Administrators
//! install an allowlisted manifest whose signature and executable digest are
//! verified before the scheduler may launch it.

use std::path::Path;

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecoderInputKind {
    Iq,
    Discriminator,
    MonoPcm,
    StereoPcm,
    WfmMultiplex,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecoderInputContract {
    pub kind: DecoderInputKind,
    pub sample_rate_hz: u32,
    pub bandwidth_hz: u32,
    pub sample_format: String,
    pub owns_tuner: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecoderResourceLimits {
    pub memory_mb: u32,
    pub cpu_percent: u16,
    pub maximum_instances: u16,
    pub restart_limit: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecoderParameter {
    pub id: String,
    pub kind: String,
    pub required: bool,
    pub default: Option<serde_json::Value>,
}

/// Fields covered by the Ed25519 signature. Field order is fixed by this Rust
/// type, giving installers a deterministic serialization to sign.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecoderManifestPayload {
    pub schema_version: u16,
    pub id: String,
    pub name: String,
    pub version: String,
    pub executable: String,
    pub executable_sha256: String,
    pub arguments: Vec<String>,
    pub input: DecoderInputContract,
    pub resources: DecoderResourceLimits,
    pub health_command: Vec<String>,
    pub output_schema: String,
    pub parameters: Vec<DecoderParameter>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedDecoderManifest {
    #[serde(flatten)]
    pub payload: DecoderManifestPayload,
    pub signer: String,
    pub signature_ed25519_base64: String,
}

impl SignedDecoderManifest {
    pub fn validate_shape(&self) -> anyhow::Result<()> {
        if self.payload.schema_version != 1 {
            anyhow::bail!("unsupported decoder manifest schema");
        }
        if self.payload.id.is_empty()
            || !self
                .payload
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            anyhow::bail!("decoder id must use lowercase letters, digits, dash, or underscore");
        }
        if self.payload.arguments.iter().any(|arg| arg.contains('\0')) {
            anyhow::bail!("decoder arguments may not contain NUL");
        }
        if self.payload.resources.memory_mb < 16
            || self.payload.resources.cpu_percent == 0
            || self.payload.resources.cpu_percent > 100
            || self.payload.resources.maximum_instances == 0
        {
            anyhow::bail!("invalid decoder resource limits");
        }
        if self.payload.executable_sha256.len() != 64
            || hex::decode(&self.payload.executable_sha256).is_err()
        {
            anyhow::bail!("executable_sha256 must be a 64-character hex digest");
        }
        Ok(())
    }

    pub fn verify_signature(&self, public_key: &VerifyingKey) -> anyhow::Result<()> {
        self.validate_shape()?;
        let bytes = serde_json::to_vec(&self.payload)?;
        let signature_bytes =
            base64::engine::general_purpose::STANDARD.decode(&self.signature_ed25519_base64)?;
        let signature = Signature::from_slice(&signature_bytes)?;
        public_key.verify(&bytes, &signature)?;
        Ok(())
    }

    pub fn verify_executable(&self, decoder_root: &Path) -> anyhow::Result<()> {
        self.validate_shape()?;
        let root = decoder_root.canonicalize()?;
        let candidate = decoder_root.join(&self.payload.executable).canonicalize()?;
        if !candidate.starts_with(&root) || !candidate.is_file() {
            anyhow::bail!("decoder executable must be a file inside the decoder root");
        }
        let digest = Sha256::digest(std::fs::read(candidate)?);
        if hex::encode(digest) != self.payload.executable_sha256.to_ascii_lowercase() {
            anyhow::bail!("decoder executable checksum mismatch");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn payload() -> DecoderManifestPayload {
        DecoderManifestPayload {
            schema_version: 1,
            id: "dmr-test".into(),
            name: "DMR fixture".into(),
            version: "1.0.0".into(),
            executable: "decoder".into(),
            executable_sha256: "00".repeat(32),
            arguments: vec!["--stdin".into()],
            input: DecoderInputContract {
                kind: DecoderInputKind::Discriminator,
                sample_rate_hz: 48_000,
                bandwidth_hz: 12_500,
                sample_format: "f32le".into(),
                owns_tuner: false,
            },
            resources: DecoderResourceLimits {
                memory_mb: 128,
                cpu_percent: 50,
                maximum_instances: 2,
                restart_limit: 3,
            },
            health_command: vec!["--version".into()],
            output_schema: "pulsescope.decoder-event.v1".into(),
            parameters: vec![],
        }
    }

    #[test]
    fn signed_manifest_detects_payload_tampering() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let payload = payload();
        let signature = key.sign(&serde_json::to_vec(&payload).unwrap());
        let mut manifest = SignedDecoderManifest {
            payload,
            signer: "fixture".into(),
            signature_ed25519_base64: base64::engine::general_purpose::STANDARD
                .encode(signature.to_bytes()),
        };
        manifest.verify_signature(&key.verifying_key()).unwrap();
        manifest.payload.input.sample_rate_hz = 24_000;
        assert!(manifest.verify_signature(&key.verifying_key()).is_err());
    }

    #[test]
    fn manifest_rejects_unsafe_identity_and_limits() {
        let mut manifest = SignedDecoderManifest {
            payload: payload(),
            signer: "fixture".into(),
            signature_ed25519_base64: String::new(),
        };
        manifest.payload.id = "../../shell".into();
        assert!(manifest.validate_shape().is_err());
        manifest.payload.id = "safe".into();
        manifest.payload.resources.cpu_percent = 101;
        assert!(manifest.validate_shape().is_err());
    }

    #[test]
    fn sidecar_templates_validate_shape() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/decoder-manifests");
        for name in ["satdump.template.json", "rs41mod.template.json"] {
            let text = std::fs::read_to_string(root.join(name)).unwrap();
            let manifest: SignedDecoderManifest = serde_json::from_str(&text).unwrap();
            manifest.validate_shape().unwrap();
            assert!(!manifest.payload.executable.is_empty());
            assert!(manifest.payload.output_schema.contains(".v1"));
        }
    }
}
