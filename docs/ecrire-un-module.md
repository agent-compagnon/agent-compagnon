# Écrire un module MCP local

Un module local est un MCP écrit **dans** l'appli, pour un service qui n'a pas de serveur MCP. Il passe par la même
chaîne qu'un MCP distant (politique, scan des arguments, limite de débit, confirmation, session, filtre de sortie,
trace) et ne manipule jamais un secret (INV-4) : il reçoit un client HTTP déjà authentifié. Exemple complet :
`crates/modules/jira-local` (spec 025).

## 1. Le crate

`crates/modules/<id>/`, ajouté aux `members` et `default-members` du `Cargo.toml` racine. Il ne dépend, parmi les
crates du dépôt, que de `mcp-proxy-core`, et n'en utilise que `mcp_proxy_core::module` (un test de
`crates/gateway/tests/local_module.rs` le vérifie sur les sources de `crates/modules/*`). Ce module réexporte ce
qu'il faut : `Method`, `Request`, `Response`, `StatusCode`, `header`, `HeaderValue`, `Url`, `Value`, `json!`,
`json_from_slice`, `async_trait`, `read_body`.

## 2. Le contrat

```rust
#[async_trait]
pub trait LocalModule: Send + Sync {
    fn id(&self) -> &'static str;               // valeur de `module` dans la config
    fn tools(&self) -> Vec<ToolDefinition>;     // noms d'origine, sans préfixe
    async fn call(&self, ctx: &ModuleContext, tool: &str, args: JsonObject) -> Result<ToolOutput, ModuleError>;
}

pub struct ModuleContext {
    pub http: AuthenticatedHttpClient, // lié à la session de l'instance et à l'origine de base_url
    pub base_url: Url,
    pub env: EnvId,
}
```

- **Requêtes** : `ctx.http.request(méthode, url)`, puis `ctx.http.execute(requête)`. L'identifiant est injecté au
  moment de l'envoi ; un en-tête `Authorization` ou `Cookie` posé par le module est remplacé. Seule l'origine de
  `base_url` est joignable ; `TRACE` et `CONNECT` sont refusés ; aucune redirection n'est suivie. Construire les URL
  à partir de `ctx.base_url` (garder un éventuel chemin de contexte, encoder chaque segment).
- **Corps de réponse** : `read_body(réponse, DEFAULT_BODY_LIMIT)` borne la taille lue.
- **401 et 403** : le client les voit lui-même, marque le secret refusé, et la passerelle relance une connexion puis
  rejoue l'appel une fois. Le module rend `ModuleError::CredentialRejected` ; il ne peut pas, à lui seul, faire
  expirer une session.
- **Erreurs** : `ModuleError::InvalidArguments` (contrôlés avant toute requête), `ModuleError::Failed` avec une
  phrase claire en français (`ticket introuvable : OPS-1`), jamais le corps brut d'une réponse. `?` sur une erreur
  du client donne `ModuleError::Http`.
- **Sortie** : `ToolOutput::text(..).with_structured(json!(..))`, limitée aux champs utiles à un LLM, textes longs
  tronqués. Tout passe ensuite par le filtre de sortie.
- **Outils** : schémas d'entrée précis (types, bornes, champs obligatoires, `additionalProperties: false`) et
  descriptions utiles à un LLM. Annotations cohérentes avec la catégorie de la config (`readOnlyHint`,
  `destructiveHint`) ; la catégorie retenue reste celle de la config.

## 3. La déclaration

1. **Exe** : ajouter le module à `mcp_proxy_gateway::modules::builtin()`, seul endroit qui liste les modules compilés.
   Un `module` absent de cette liste est une erreur de validation de la config.
2. **Config** (`config/default.toml`) :

   ```toml
   [[mcp]]
   id = "jira"
   alias = "jira"
   label = "Jira"
   kind = "local"
   module = "jira-local"
   session = "sso-tools"          # l'origine de base_url s'ajoute aux origines de la session
   [mcp.instances]
   prod = { base_url = "https://jira.example.internal" }
   [mcp.tools]
   search_issues = { category = "read" }
   add_comment = { category = "write" }
   ```

   Chaque outil du module est déclaré dans `[mcp.tools]` avec sa catégorie ; un outil non déclaré est inconnu, donc
   traité comme destructif.
3. **Instantané** : `config/catalog/<mcp>.json` contient exactement `tools()` (outils connus avant toute connexion,
   spec 005). Un test unitaire du module compare les deux.

## 4. Les tests

- **Unitaires**, dans le crate : contrôle des arguments, rendu des sorties, traduction des statuts HTTP, égalité
  avec l'instantané.
- **De bout en bout**, dans `crates/gateway/tests/` : vrai client MCP → passerelle → chaîne → `LocalUpstream` →
  module → faux service du testkit (modèle : `FakeJira`, qui contrôle l'en-tête d'authentification et enregistre les
  requêtes). La config embarquée est pointée vers le faux service en remplaçant l'hôte dans `EMBEDDED_TOML`, la
  session est ouverte par le vrai `OnDemandAuthenticator` avec `FakeAuthUi`. À couvrir : outils exposés avec leur
  préfixe et leur catégorie, requête HTTP exacte, confirmation exigée là où la politique le dit, secret masqué dans
  une réponse, 401 suivi d'une reconnexion et d'un seul rejeu, erreurs claires.
- **Recette** : une ligne dans `docs/recette.md` pour l'essai contre le vrai service sous Windows.
