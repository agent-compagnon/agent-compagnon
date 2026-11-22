---
id: "006"
titre: "gateway — transports MCP (côté clients, côté serveurs) et testkit"
etape: "3b"
statut: "prévue"
depend_de: ["002", "003", "005"]
suivi: "github#7"
---

# 006 — gateway — transports MCP (côté clients, côté serveurs) et testkit

## Objectif

Faire transiter un `tools/list` et un `tools/call` de bout en bout : client LLM → passerelle → serveur MCP, dans
les deux générations du protocole, avec injection des identifiants côté serveur.

## À faire

### Testkit (`crates/testkit`)

- `MockMcpServer` (rmcp, feature `transport-streamable-http-server`), configurable :
  - jeu d'outils ; changement de liste à chaud, avec ou sans notification ;
  - en-tête exigé (nom + valeur), sinon 401, avec ou sans `WWW-Authenticate` ;
  - génération du protocole : ancienne, 2026-07-28, ou les deux ;
  - outils spéciaux : `echo_headers` (renvoie les en-têtes reçus, pour éprouver le filtre), `slow`, `fail` ;
  - une route de redirection vers une autre origine ;
  - journal des requêtes reçues, pour les assertions.
- `FakeSessionGate` (identifiant fixe ou états scriptés) et `FakeConfirmationBroker`.

### Contrats (dans `core`)

```rust
#[async_trait]
pub trait SessionGate: Send + Sync {
    /// Rend la session utilisable avant `deadline`, en déclenchant une connexion si nécessaire.
    async fn ensure(&self, session: &SessionId, deadline: Instant, reason: &AccessReason) -> Result<(), SessionError>;
    /// Ajoute les en-têtes d'authentification, après contrôle de l'origine.
    fn inject(&self, session: &SessionId, url: &Url, headers: &mut HeaderMap) -> Result<(), SessionError>;
    /// Le serveur a refusé l'identifiant : la session passe à « expirée ».
    fn mark_rejected(&self, session: &SessionId);
}
```

### Côté serveurs : `Upstream`

- Trait `Upstream` : `list_tools`, `call_tool`, flux de changements. Deux implémentations à terme : `HttpUpstream`
  (ici) et les modules locaux (étape 8).
- `HttpUpstream` : client rmcp lancé avec `serve_with_lifecycle(.., ClientLifecycleMode::Auto { .. })`, donc
  compatible avec les deux générations.
- `InjectingHttpClient` : type qui implémente le trait `StreamableHttpClient` de rmcp autour de `reqwest::Client`.
  - À chaque requête, il appelle `SessionGate::inject` **au moment de l'envoi**. Aucun en-tête d'authentification
    n'est stocké dans la configuration du transport.
  - Aucune redirection suivie. TLS natif.
  - 401 ou 403 → `UpstreamError::AuthRejected` et `mark_rejected`, qu'il y ait ou non un `WWW-Authenticate`.
  - Les limites de taille de réponse sont conservées (redéfinir les méthodes `*_with_response_limits`).
- Connexion paresseuse au premier besoin ; reconnexion avec temporisation croissante.
- Délai d'appel `upstream_timeout_secs` via `PeerRequestOptions` ; notifications de progression relayées vers le
  client quand il a fourni un jeton de progression.
- Sans session active, `list_tools` renvoie `UpstreamError::SessionRequired`, sauf si `list_requires_auth = false`.

### Côté clients : `GatewayServer`

- Implémente `rmcp::ServerHandler`. Capacités annoncées : **outils seulement** (`list_changed` inclus). Ressources,
  prompts, sampling, roots : non gérés.
- `list_tools` : outils exposés du registre, avec `ttl_ms = list_ttl_secs`.
- `call_tool` : délègue à un `CallPipeline`. Cette étape livre le pipeline minimal : résolution → `ensure` →
  appel amont → **filtre de sortie**. L'étape 3c y insère les autres maillons.
- `ClientIdentity { name, version, protocol_version, supports_elicitation }`, lue par requête depuis
  `RequestContext` (`client_info()`, `protocol_version()`, `client_capabilities()`).
- `GatewayServer::serve(stream)` pour tout `AsyncRead + AsyncWrite`.
- Erreurs renvoyées comme résultat d'outil en erreur (`is_error = true`) avec un message français clair, sans
  secret. Les messages venant de l'amont passent par le filtre : `UnexpectedServerResponse` contient le corps de la
  réponse.

### Pont

- `gateway::bridge::relay(stdin, stdout, pipe)` : relais d'octets générique entre deux flux asynchrones, utilisé par
  le mode `stdio` de l'appli (5a).

### Exemple de développement

- `crates/gateway/examples/gateway_stdio.rs` : lance la passerelle sur stdio avec la config embarquée et les faux
  serveurs du testkit. Permet d'essayer un vrai client MCP sur n'importe quel OS avant que l'appli existe.

### Instantanés de catalogue

- `cargo xtask snapshot-catalog --url <url> --out config/catalog/<mcp>.json [--header-env NOM]` : interroge un
  serveur MCP (`tools/list`, toutes les pages) et écrit l'instantané au format du catalogue embarqué de la spec 005
  (tableau de `ToolDefinition`). L'en-tête d'authentification vient d'une variable d'environnement et n'est jamais
  affiché, ni dans la sortie, ni dans une erreur.
- `xtask` reste sans dépendance lourde : le travail est fait par un exécutable du crate `gateway` (qui a déjà le
  client MCP), que `xtask` lance avec `cargo run`.

## Tests

- **EX-006-01** — Matrice (client ancien, client 2026-07-28) × (serveur ancien, serveur 2026-07-28) : `tools/list` puis
  `tools/call`.
- **EX-006-02** — L'en-tête d'authentification arrive au bon serveur, et à lui seul ; il n'apparaît dans aucune réponse.
- **EX-006-03** — 401 → `AuthRejected` et session marquée ; redirection vers une autre origine non suivie.
- **EX-006-04** — Outil inconnu ou non exposé : absent de la liste, appel refusé.
- **EX-006-05** — `echo_headers` : le jeton renvoyé par le serveur est masqué dans la réponse reçue par le client.
- **EX-006-06** — Corps d'erreur amont contenant le jeton : masqué.
- **EX-006-07** — `relay` : octets transmis dans les deux sens, fin de flux propagée.
- **EX-006-08** — `snapshot-catalog` contre le faux serveur : le fichier écrit est relu par le catalogue embarqué de la
  spec 005 ; l'en-tête lu dans la variable d'environnement arrive au serveur et n'apparaît ni dans la sortie ni dans
  les messages d'erreur.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Politiques, confirmations, limite de débit (3c). Rechargement des listes (3d).

## Amendements

- **2026-11-22** — Ajout de `cargo xtask snapshot-catalog` (section « Instantanés de catalogue ») et du critère
  `EX-006-08`, venus de la spec 005. Raison : la commande a besoin du client MCP HTTP livré ici ; `xtask` délègue à
  un exécutable du crate `gateway` pour rester sans dépendance lourde.
