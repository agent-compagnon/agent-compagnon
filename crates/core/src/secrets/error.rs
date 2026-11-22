use thiserror::Error;

use crate::config::SessionId;

/// Why the memory protection layer failed. Carries no data from the secret nor from the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionFailure {
    /// The value is too large to be sealed.
    TooLarge,
    /// The operating system's random generator did not answer.
    Random,
    /// The key wrapper gave back a key of the wrong length.
    InvalidKey,
    /// The trial round trip of the key wrapper did not give the key back.
    RoundTrip,
    /// Error code returned by the operating system (`HRESULT` on Windows).
    Os(i32),
}

/// Errors of the secret store, the origin guard and the authenticated HTTP client.
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

    /// Returned when a store is created: the session key cannot be wrapped and given back.
    #[error(
        "protection des secrets indisponible ({0}) : la clé de session ne peut pas être protégée, aucun secret ne \
         sera gardé"
    )]
    KeyProtectionUnavailable(ProtectionFailure),

    #[error("secret illisible après déchiffrement")]
    Corrupted,

    #[error("destination refusée pour la session « {session} » : {origin} n'est pas une origine autorisée")]
    OriginNotAllowed { session: SessionId, origin: String },

    #[error("destination refusée pour la session « {session} » : https obligatoire")]
    InsecureScheme { session: SessionId },

    #[error("destination refusée pour la session « {session} » : l'URL contient des identifiants")]
    CredentialsInUrl { session: SessionId },

    #[error("client HTTP : initialisation impossible")]
    ClientInit,

    #[error("requête HTTP : délai dépassé")]
    Timeout,

    #[error("requête HTTP : connexion impossible")]
    Connect,

    #[error("requête HTTP : échec de l'échange")]
    Transport,
}

impl std::fmt::Display for ProtectionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => f.write_str("valeur trop grande"),
            Self::Random => f.write_str("générateur aléatoire indisponible"),
            Self::InvalidKey => f.write_str("clé de session de taille inattendue"),
            Self::RoundTrip => f.write_str("aller-retour de la clé de session incorrect"),
            Self::Os(code) => write!(f, "code système {code:#010x}"),
        }
    }
}
