# [Étape 8] Module MCP local d'exemple (`jira-local`)

> Référence commune : `00-cadrage.md`. Dépend de : étapes 2a, 3c, 4.

## Objectif

Montrer comment écrire un MCP **dans** l'appli, pour un service qui n'a pas de serveur MCP, sans que le module
touche jamais à un secret.

## À faire

### Contrat des modules

```rust
#[async_trait]
pub trait LocalModule: Send + Sync {
    fn id(&self) -> &'static str;
    fn tools(&self) -> Vec<ToolDefinition>;
    async fn call(&self, ctx: &ModuleContext, tool: &str, args: JsonObject) -> Result<ToolOutput, ModuleError>;
}

pub struct ModuleContext {
    pub http: AuthenticatedHttpClient, // déjà lié à la session et à ses origines
    pub base_url: Url,
    pub env: EnvId,
}
```

- Le contrat vit dans une API étroite du core (`mcp_proxy_core::module`). Un crate de module ne dépend que d'elle :
  il n'a aucun accès au `SecretStore`.
- `AuthenticatedHttpClient` n'offre aucun moyen de lire les en-têtes injectés (2a) : la garantie est portée par
  les types.
- `LocalUpstream` implémente `Upstream` (3b) pour une instance de module. Un module local passe donc par **la même
  chaîne** qu'un MCP distant : politique, scan des arguments, limite de débit, confirmation, session, filtre de
  sortie, trace.
- Registre des modules compilés dans l'exe ; `kind = "local"` et `module = "<id>"` dans la config ; un module
  inconnu est une erreur de validation.

### `crates/modules/jira-local`

| Outil | Catégorie | Appel Jira |
|---|---|---|
| `search_issues(jql, max_results)` | lecture | `GET /rest/api/2/search` |
| `get_issue(key)` | lecture | `GET /rest/api/2/issue/{key}` |
| `add_comment(key, body)` | écriture | `POST /rest/api/2/issue/{key}/comment` |

- Schémas d'entrée précis (types, bornes, champs obligatoires) et descriptions utiles à un LLM.
- Sortie concise : texte lisible et contenu structuré, limités aux champs utiles.
- Erreurs HTTP traduites en messages clairs (`ticket introuvable`, `requête JQL invalide`), sans recopier le corps
  brut.
- Un 401 ou 403 est remonté comme identifiant rejeté : la session expire et la reconnexion à la demande s'applique
  comme pour un MCP distant.

### Testkit et documentation

- `FakeJira` : contrôle l'en-tête d'authentification, sert des réponses d'exemple, enregistre les requêtes.
- `docs/ecrire-un-module.md` : guide court pour ajouter un module (contrat, déclaration dans la config, tests).

## Tests

- Les outils du module apparaissent avec le préfixe d'environnement et la bonne catégorie.
- `search_issues` : requête HTTP correcte, en-tête d'authentification présent côté `FakeJira`.
- Le module ne reçoit jamais le secret : `Debug` du contexte sans valeur sensible, et aucune API ne permet de
  l'obtenir.
- `add_comment` en PROD exige une confirmation ; refus → aucune requête envoyée à Jira.
- Réponse de Jira contenant un secret connu : masquée.
- 401 → reconnexion → rejeu unique. 404 → message clair.
- Requête vers un autre hôte que `base_url` : refusée par le client authentifié.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- Avec une instance Jira réelle et la session Basic : rechercher des tickets depuis un client LLM ; ajouter un
  commentaire en PROD déclenche la confirmation.
