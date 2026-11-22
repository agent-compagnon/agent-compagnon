//! The proof of the named pipe sealed by user-scoped DPAPI (WinRT, no `unsafe`).

use mcp_proxy_core::platform::windows::{protect, unprotect};
use mcp_proxy_gateway::bridge::{PeerProof, ProtectFailure};
use zeroize::Zeroizing;

pub struct DpapiProof;

impl PeerProof for DpapiProof {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectFailure> {
        protect(data).map_err(|e| ProtectFailure(e.0))
    }

    fn unprotect(&self, data: &[u8]) -> Result<Zeroizing<Vec<u8>>, ProtectFailure> {
        unprotect(data).map_err(|e| ProtectFailure(e.0))
    }
}
