mod checks;
mod key;
mod path;

pub(crate) use checks::{check_content_hash, check_integrity_mac, check_seq_matches};
pub(crate) use key::load_or_create_integrity_key;
pub(crate) use path::claim_seq_from_path;

use crate::models::Claim;

fn canonical_claim_bytes(claim: &Claim) -> Vec<u8> {
    let mut to_serialize = claim.clone();
    to_serialize.content_hash = None;
    to_serialize.integrity_mac = None;
    serde_json::to_vec(&to_serialize)
        .expect("Claim serializes (Serialize is derived)")
}

/// canonical content hash: serialize the claim with integrity fields blanked,
/// then blake3. this remains a cheap public fingerprint for diagnostics and
/// drift reporting, but it is NOT the authority for tamper-resistance.
pub(super) fn canonical_content_hash(claim: &Claim) -> String {
    blake3::hash(&canonical_claim_bytes(claim)).to_hex().to_string()
}

/// keyed MAC over the canonical claim bytes. unlike content_hash, this cannot
/// be forged by someone who can merely edit the json file; they also need the
/// signing key material.
pub(super) fn canonical_integrity_mac(claim: &Claim, key: &[u8; 32]) -> String {
    blake3::keyed_hash(key, &canonical_claim_bytes(claim))
        .to_hex()
        .to_string()
}

