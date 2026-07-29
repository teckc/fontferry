use std::{fs, path::Path};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use fontferry_core::{Catalog, FontFerryError, Result};

#[derive(Clone, Debug)]
pub struct CatalogVerifier {
    public_key: VerifyingKey,
}

impl CatalogVerifier {
    pub fn from_base64(public_key: &str) -> Result<Self> {
        let bytes = STANDARD
            .decode(public_key.trim())
            .map_err(|_| FontFerryError::InvalidCatalogSignature)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| FontFerryError::InvalidCatalogSignature)?;
        let public_key = VerifyingKey::from_bytes(&bytes)
            .map_err(|_| FontFerryError::InvalidCatalogSignature)?;
        Ok(Self { public_key })
    }

    pub fn verify(&self, body: &[u8], signature: &str) -> Result<Catalog> {
        let signature = STANDARD
            .decode(signature.trim())
            .map_err(|_| FontFerryError::InvalidCatalogSignature)?;
        let signature = Signature::from_slice(&signature)
            .map_err(|_| FontFerryError::InvalidCatalogSignature)?;
        self.public_key
            .verify(body, &signature)
            .map_err(|_| FontFerryError::InvalidCatalogSignature)?;
        parse_catalog(body)
    }
}

pub fn parse_catalog(body: &[u8]) -> Result<Catalog> {
    let catalog: Catalog = serde_json::from_slice(body)
        .map_err(|error| FontFerryError::InvalidCatalog(error.to_string()))?;
    catalog.validate()?;
    Ok(catalog)
}

pub fn load_embedded_or_cached(
    embedded: &[u8],
    cached_body: &Path,
    cached_signature: &Path,
    verifier: Option<&CatalogVerifier>,
) -> Result<Catalog> {
    if let Some(verifier) = verifier
        && let (Ok(body), Ok(signature)) = (
            fs::read(cached_body),
            fs::read_to_string(cached_signature),
        )
        && let Ok(catalog) = verifier.verify(&body, &signature)
    {
        return Ok(catalog);
    }
    parse_catalog(embedded)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    #[test]
    fn rejects_tampered_catalog() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let signing = SigningKey::from_bytes(&[7_u8; 32]);
        let verifier = CatalogVerifier {
            public_key: signing.verifying_key(),
        };
        let body = br#"{"schemaVersion":1}"#;
        let signature = STANDARD.encode(signing.sign(body).to_bytes());
        assert!(verifier.verify(body, &signature).is_err());
        assert!(verifier.verify(b"tampered", &signature).is_err());
        Ok(())
    }
}

