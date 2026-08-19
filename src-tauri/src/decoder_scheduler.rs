//! Signed manifest discovery, verification, and supervised decoder scheduling.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::VerifyingKey;
use tokio::sync::broadcast;

use crate::db::Db;
use crate::decoder_manifest::SignedDecoderManifest;
use crate::sidecar::SidecarRegistry;
use crate::state::ScannerEvent;

#[derive(Clone, Default)]
pub struct DecoderScheduler {
    trusted_keys: Vec<VerifyingKey>,
}

impl DecoderScheduler {
    pub fn with_trusted_keys(keys: Vec<VerifyingKey>) -> Self {
        Self { trusted_keys: keys }
    }

    pub fn load_trusted_keys(data_dir: &Path) -> Vec<VerifyingKey> {
        let mut keys = Vec::new();
        if let Ok(raw) = std::env::var("PULSESCOPE_DECODER_TRUSTED_PUBLIC_KEY_B64") {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw.trim()) {
                if let Ok(key) =
                    VerifyingKey::from_bytes(bytes.as_slice().try_into().unwrap_or(&[0; 32]))
                {
                    keys.push(key);
                }
            }
        }
        let path = data_dir.join("decoders/trusted_public_keys.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(list) = serde_json::from_str::<Vec<String>>(&text) {
                for entry in list {
                    if let Ok(bytes) =
                        base64::engine::general_purpose::STANDARD.decode(entry.trim())
                    {
                        if let Ok(arr) = bytes.as_slice().try_into() {
                            if let Ok(key) = VerifyingKey::from_bytes(arr) {
                                keys.push(key);
                            }
                        }
                    }
                }
            }
        }
        keys
    }

    pub fn decoder_root(data_dir: &Path) -> PathBuf {
        data_dir.join("decoders")
    }

    pub fn manifest_dir(data_dir: &Path) -> PathBuf {
        Self::decoder_root(data_dir).join("manifests")
    }

    pub fn scan_manifests(dir: &Path) -> HashMap<String, SignedDecoderManifest> {
        let mut out = HashMap::new();
        if !dir.is_dir() {
            return out;
        }
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if let Ok(manifest) = serde_json::from_str::<SignedDecoderManifest>(&text) {
                out.insert(manifest.payload.id.clone(), manifest);
            }
        }
        out
    }

    pub fn verify_manifest(
        &self,
        manifest: &SignedDecoderManifest,
        decoder_root: &Path,
    ) -> anyhow::Result<()> {
        if self.trusted_keys.is_empty() {
            anyhow::bail!("no trusted decoder signing keys configured");
        }
        let mut verified = false;
        for key in &self.trusted_keys {
            if manifest.verify_signature(key).is_ok() {
                verified = true;
                break;
            }
        }
        if !verified {
            anyhow::bail!("decoder manifest signature is not trusted");
        }
        manifest.verify_executable(decoder_root)?;
        Ok(())
    }

    pub async fn spawn_verified(
        &self,
        sidecars: &SidecarRegistry,
        manifest: &SignedDecoderManifest,
        decoder_root: &Path,
        db: Db,
        events_tx: broadcast::Sender<ScannerEvent>,
    ) -> anyhow::Result<()> {
        self.verify_manifest(manifest, decoder_root)?;
        for key in &self.trusted_keys {
            if manifest.verify_signature(key).is_ok() {
                return sidecars
                    .spawn_manifest_decoder(manifest, decoder_root, key, db, events_tx)
                    .await;
            }
        }
        anyhow::bail!("trusted key verification failed")
    }

    pub async fn sync_manifest_jobs(
        &self,
        sidecars: &SidecarRegistry,
        data_dir: &Path,
        enabled_ids: &[&str],
        db: Db,
        events_tx: broadcast::Sender<ScannerEvent>,
    ) {
        let root = Self::decoder_root(data_dir);
        let manifests = Self::scan_manifests(&Self::manifest_dir(data_dir));
        for &id in enabled_ids {
            if sidecars.is_running(id) {
                continue;
            }
            if let Some(manifest) = manifests.get(id) {
                match self
                    .spawn_verified(sidecars, manifest, &root, db.clone(), events_tx.clone())
                    .await
                {
                    Ok(()) => tracing::info!(decoder = id, "manifest decoder started"),
                    Err(error) => {
                        tracing::warn!(decoder = id, error = %error, "manifest decoder failed to start")
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    fn fixture_manifest(
        signing_key: &SigningKey,
        id: &str,
        executable: &str,
        digest: &str,
    ) -> SignedDecoderManifest {
        use crate::decoder_manifest::{
            DecoderInputContract, DecoderInputKind, DecoderManifestPayload, DecoderResourceLimits,
        };
        let payload = DecoderManifestPayload {
            schema_version: 1,
            id: id.into(),
            name: id.into(),
            version: "1.0.0".into(),
            executable: executable.into(),
            executable_sha256: digest.into(),
            arguments: vec!["--version".into()],
            input: DecoderInputContract {
                kind: DecoderInputKind::Iq,
                sample_rate_hz: 2_000_000,
                bandwidth_hz: 2_000_000,
                sample_format: "cf32".into(),
                owns_tuner: false,
            },
            resources: DecoderResourceLimits {
                memory_mb: 128,
                cpu_percent: 50,
                maximum_instances: 1,
                restart_limit: 3,
            },
            health_command: vec!["--version".into()],
            output_schema: "pulsescope.decoder-event.v1".into(),
            parameters: vec![],
        };
        let signature = signing_key.sign(&serde_json::to_vec(&payload).unwrap());
        SignedDecoderManifest {
            payload,
            signer: "fixture".into(),
            signature_ed25519_base64: base64::engine::general_purpose::STANDARD
                .encode(signature.to_bytes()),
        }
    }

    #[test]
    fn scan_and_verify_manifest_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "pulsescope-decoder-manifests-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let decoder_root = dir.join("root");
        std::fs::create_dir_all(&decoder_root).unwrap();
        let exe_path = decoder_root.join("decoder.sh");
        std::fs::write(&exe_path, "#!/bin/sh\nexit 0\n").unwrap();
        let digest = Sha256::digest(std::fs::read(&exe_path).unwrap());
        let digest_hex = hex::encode(digest);

        let key = SigningKey::from_bytes(&[9; 32]);
        let manifest = fixture_manifest(&key, "rtl_433", "decoder.sh", &digest_hex);
        std::fs::write(
            dir.join("rtl_433.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let scheduler = DecoderScheduler::with_trusted_keys(vec![key.verifying_key()]);
        let loaded = DecoderScheduler::scan_manifests(&dir);
        assert_eq!(loaded.len(), 1);
        scheduler
            .verify_manifest(loaded.get("rtl_433").unwrap(), &decoder_root)
            .unwrap();
    }
}
