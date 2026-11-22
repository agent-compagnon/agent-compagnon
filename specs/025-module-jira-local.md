---
id: "025"
titre: "Module MCP local d'exemple (`jira-local`)"
etape: "8"
statut: "implémentée"
depend_de: ["002", "007", "010"]
suivi: "github#26"
---

# 025 — Module MCP local d'exemple (`jira-local`)

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
    pub http: AuthenticatedHttpClient, // lié à la session de l'instance et à la seule origine de base_url
    pub base_url: Url,
    pub env: EnvId,
}
```

- Le contrat vit dans une API étroite du core (`mcp_proxy_core::module`) : `LocalModule`, `ModuleContext`,
  `ToolOutput` (texte et contenu structuré), `ModuleError` (`UnknownTool`, `InvalidArguments`,
  `CredentialRejected`, `Http`, `Failed`, messages en français), `ModuleRegistry`, `read_body` (corps borné), et la
  réexportation de ce dont un module a besoin (`Method`, `Request`, `Response`, `StatusCode`, `header`, `Url`,
  `Value`, `json!`, `async_trait`). `ToolDefinition` et `ToolHints` y vivent désormais, réexportés par le registre
  de la passerelle. Un crate de module ne dépend, parmi les crates du dépôt, que du core et n'en utilise que ce
  module : il n'a aucun accès au `SecretStore`.
- `AuthenticatedHttpClient` n'offre aucun moyen de lire les en-têtes injectés, le secret ni sa version (2a) : la
  garantie est portée par les types. Pour les modules, il injecte **par le `SessionGate`**
  (`AuthenticatedHttpClient::through_gate`), comme les MCP distants : l'échéance de la session et l'authentification
  à la demande s'appliquent. Il est **restreint à l'origine de `base_url`** (`only_towards`), même si la session a
  d'autres origines ; il refuse `TRACE` (qui renverrait les en-têtes envoyés) et `CONNECT` avant tout envoi.
- Un 401 ou un 403 est vu par le client lui-même : il marque refusé le secret qu'il a envoyé
  (`SessionGate::mark_rejected` avec sa version, que le module ne voit jamais) et incrémente un `RejectionCounter`
  remis à celui qui a construit le client, pas au module. Le module ne peut donc ni cacher un refus, ni faire
  expirer une session à lui seul.
- `LocalUpstream` implémente `Upstream` (3b) pour une instance de module. Un module local passe donc par **la même
  chaîne** qu'un MCP distant : politique, scan des arguments, limite de débit, confirmation, session, filtre de
  sortie, trace. Un refus vu par le client donne `UpstreamError::AuthRejected` (nouvelle connexion, un seul rejeu) ;
  une erreur du module devient un résultat d'outil en erreur ; un délai, une connexion impossible ou une destination
  refusée deviennent les erreurs amont correspondantes. L'appel est borné par `upstream_timeout_secs`.
- Registre des modules compilés dans l'exe : `mcp_proxy_gateway::modules::builtin()`, seul endroit qui les liste ;
  `embedded_config()` valide la config embarquée avec leurs identifiants. Le core ne connaît aucun module
  (`Config::embedded(known_modules)`). `kind = "local"` et `module = "<id>"` dans la config ; un module inconnu est
  une erreur de validation. `Upstreams::from_config` crée un `LocalUpstream` par instance de module local.

### `crates/modules/jira-local`

| Outil | Catégorie | Appel Jira |
|---|---|---|
| `search_issues(jql, max_results)` | lecture | `GET /rest/api/2/search` |
| `get_issue(key)` | lecture | `GET /rest/api/2/issue/{key}` |
| `add_comment(key, body)` | écriture | `POST /rest/api/2/issue/{key}/comment` |

- Schémas d'entrée précis (types, bornes, champs obligatoires, `additionalProperties: false`) et descriptions utiles
  à un LLM : `jql` de 1 à 2000 caractères ; `max_results` de 1 à 50, 20 par défaut ; `key` de la forme `OPS-123`
  (mis en majuscules) ; `body` de 1 à 32 767 caractères. Les arguments sont contrôlés avant toute requête.
- Sortie concise : texte lisible et contenu structuré, limités aux champs utiles (clé, résumé, statut, type,
  priorité, assigné, dates ; pour `get_issue`, rapporteur, description tronquée à 4000 caractères et les 5 derniers
  commentaires tronqués à 1000). Corps de réponse limité à 4 Mio.
- Erreurs HTTP traduites en messages clairs (`ticket introuvable`, `requête JQL invalide`, Jira indisponible, débit
  limité, réponse inattendue avec le seul code HTTP), sans recopier le corps brut.
- Un 401 ou 403 est remonté comme identifiant rejeté : la session expire et la reconnexion à la demande s'applique
  comme pour un MCP distant.
- Les outils sont exactement l'instantané embarqué `config/catalog/jira.json` et les outils déclarés dans la config
  d'exemple.

### Testkit et documentation

- `FakeJira` : contrôle l'en-tête d'authentification (Basic, identifiants changeables pour simuler un mot de passe
  modifié), sert des réponses d'exemple (dont la sonde `myself`), enregistre les requêtes.
- `docs/ecrire-un-module.md` : guide court pour ajouter un module (contrat, déclaration dans la config, tests).
- L'exemple `gateway_stdio` sert `jira` par le module, contre un `FakeJira`.

## Tests

- **EX-025-01** — Les outils du module apparaissent avec le préfixe d'environnement et la bonne catégorie.
- **EX-025-02** — `search_issues` : requête HTTP correcte, en-tête d'authentification présent côté `FakeJira`.
- **EX-025-03** — Le module ne reçoit jamais le secret : `Debug` du contexte sans valeur sensible, et aucune API ne permet de
  l'obtenir.
- **EX-025-04** — `add_comment` en PROD exige une confirmation ; refus → aucune requête envoyée à Jira.
- **EX-025-05** — Réponse de Jira contenant un secret connu : masquée.
- **EX-025-06** — 401 → reconnexion → rejeu unique. 404 → message clair.
- **EX-025-07** — Requête vers un autre hôte que `base_url` : refusée par le client authentifié.
- **EX-025-09** — Le registre des modules de l'exe est la seule liste : la config embarquée se valide avec lui, un
  `module` inconnu est une erreur de validation, deux modules de même identifiant sont refusés.
- **EX-025-10** — Le client authentifié refuse `TRACE` et `CONNECT` avant tout envoi ; un 401 ou un 403 marque
  refusé le secret envoyé ; un « identifiant refusé » annoncé par le module sans 401 ni 403 n'expire pas la session.
- **EX-025-11** — Les outils du module sont exactement ceux de `config/catalog/jira.json` et ceux déclarés dans la
  config d'exemple.
- **EX-025-12** — Un crate de `crates/modules/` ne dépend d'aucun autre crate du dépôt que le core et n'en utilise
  que `mcp_proxy_core::module`.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-025-08** (manuel) — Avec une instance Jira réelle et la session Basic : rechercher des tickets depuis un client LLM ; ajouter un
  commentaire en PROD déclenche la confirmation.

## Amendements

- **2026-11-22** — Réalisation (I025.1 à I025.12). Le client du contexte injecte par le `SessionGate` (et non plus
  directement depuis le store) pour que l'échéance et l'authentification à la demande s'appliquent comme aux MCP
  distants ; il est restreint à l'origine de `base_url` et non plus à toutes les origines de la session ; il refuse
  `TRACE` et `CONNECT`. Les 401 et 403 sont détectés par le client, pas par le module : un module ne peut ni cacher
  un refus ni expirer une session. `ToolDefinition` et `ToolHints` passent du registre de la passerelle au core
  (`mcp_proxy_core::module`), pour que le contrat ne dépende pas de la passerelle. La constante provisoire
  `BUILTIN_MODULES` du core est remplacée par le registre `mcp_proxy_gateway::modules::builtin()`. L'instantané
  `config/catalog/jira.json` est réaligné sur le module (descriptions, bornes, `max_results` plafonné à 50 au lieu de
  100). Critères ajoutés : `EX-025-09` à `EX-025-12`.
