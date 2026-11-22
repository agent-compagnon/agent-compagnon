use thiserror::Error;

use crate::config::SessionId;

/// Why the memory protection layer failed. Carries no data from the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionFailure {
    /// The buffer length is not a multiple of `PROTECT_BLOCK_SIZE`.
    Misaligned,
    /// The buffer is too large for the platform API.
    TooLarge,
    /// Error code returned by the operating system (`HRESULT` on Windows).
    Os(i32),
}

/// Errors of the secret store and the origin guard.
///
/// No variant carries a secret value, nor anything derived from one: messages are in French and may reach the user
/// or the LLM as-is.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SecretError {
    #[error("aucun secret pour la session « {0} » : une connexion est nécessaire")]
    NoSecret(SessionId),

    #[error("session « {0} » inconnue : aucune origine autorisée")]
    UnknownSession(SessionId),

    /// The reason is a fixed text: it never quotes the rejected value.
    #[error("identifiants refusés : {0}")]
    InvalidCredential(&'static str),

    #[error("protection mémoire du secret impossible ({0})")]
    Protection(ProtectionFailure),

    #[error("secret illisible après déchiffrement")]
    Corrupted,

    #[error("destination refusée pour la session « {session} » : {origin} n'est pas une origine autorisée")]
    OriginNotAllowed { session: SessionId, origin: String },

    #[error("destination refusée pour la session « {session} » : https obligatoire")]
    InsecureScheme { session: SessionId },

    #[error("destination refusée pour la session « {session} » : l'URL contient des identifiants")]
    CredentialsInUrl { session: SessionId },
}

impl std::fmt::Display for ProtectionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Misaligned => f.write_str("tampon mal aligné"),
            Self::TooLarge => f.write_str("tampon trop grand"),
            Self::Os(code) => write!(f, "code système {code:#010x}"),
        }
    }
}
