---
id: "010"
titre: "auth — méthodes d'authentification et authentification à la demande"
etape: "4"
statut: "en cours"
depend_de: ["002", "006", "007"]
suivi: "github#11"
---

# 010 — auth — méthodes d'authentification et authentification à la demande

## Objectif

Obtenir les secrets au bon moment : jamais avant qu'un LLM en ait besoin, toujours de façon visible, et faire
attendre puis rejouer les appels pendant que l'utilisateur se connecte.

## À faire

### Sessions (dans `core`)

- Une session est indépendante des MCP : 2 SSO globaux, 1 SSO par environnement. Chaque MCP déclare la sienne ; une
  connexion débloque tous les MCP qui la partagent.
- `SessionState` : `NeverConnected`, `Active { until }`, `Expired`, `LoginRequested { since }`. Libellés :
  « jamais connectée », « active jusqu'à HH:MM », « expirée », « connexion demandée ».
- `SessionRegistry` : état de chaque session, services couverts (MCP qui l'utilisent), diffusion des changements.
- Une session est considérée expirée 60 s avant son échéance. À l'échéance : état `Expired` et secret effacé.
  **Aucun renouvellement n'est tenté** tant qu'un appel ne le demande pas.

### Interface vers l'appli

```rust
#[async_trait]
pub trait AuthUi: Send + Sync {
    async fn login_window(&self, spec: LoginWindowSpec) -> Result<Box<dyn LoginWindow>, AuthUiError>;
    async fn basic_prompt(&self, spec: BasicPromptSpec) -> Result<Option<BasicInput>, AuthUiError>; // None = annulé
    async fn notify_login_needed(&self, session: &SessionInfo, reason: &AccessReason);
    async fn clear_site_data(&self, hosts: &[String]) -> Result<(), AuthUiError>;
}

#[async_trait]
pub trait LoginWindow: Send {
    async fn next_event(&mut self) -> Option<LoginEvent>;   // HeaderCaptured { url, value }, Navigated { url }, Closed
    async fn cookies(&self, url: &Url) -> Result<Vec<CookiePair>, AuthUiError>;
    async fn close(self: Box<Self>);
}
```

L'appli l'implémente avec WebView2 (5b). Le testkit fournit `FakeAuthUi`. Une fenêtre de login est **toujours
visible** : `LoginWindowSpec` ne permet pas d'en créer une cachée.

### Trait `AuthMethod` et les trois méthodes

- `JwtHeader` — capture d'un JWT dans les en-têtes des requêtes de la WebView.
  - Ne retient que les requêtes dont l'URL correspond à `capture_url_pattern`, l'en-tête à `capture_header` et le
    schéma à `capture_scheme`.
  - Valide la structure (3 segments, en-tête et charge utile JSON), `exp` dans le futur, `aud` et `iss` s'ils sont
    attendus. La signature n'est pas vérifiée : ce n'est pas le rôle du proxy.
  - Échéance = `min(exp, maintenant + max_ttl_hours)`. Premier jeton valide retenu, puis fermeture.
- `Cookie` — cookie de session lu dans le gestionnaire de cookies.
  - Après une navigation correspondant à `success_url_pattern`, lit les cookies de `cookie_url` et exige tous les
    `cookie_names`.
  - Échéance = `min(expiration du cookie, maintenant + ttl_hours)`.
- `Basic` — login et mot de passe saisis dans une fenêtre de l'appli.
  - Si `probe_url` est défini : requête de contrôle. 401 ou 403 → « identifiants refusés », nouvelle saisie, 3
    essais au plus.
  - Échéance = `maintenant + ttl_hours`, plafonnée à 24 h.
- Le trait est extensible : ajouter une méthode ne touche ni la passerelle ni le core.
- L'injection (`Authorization: Bearer`, `Cookie`, `Authorization: Basic`) reste dans le core (2a).

### Authentification à la demande

`OnDemandAuthenticator` implémente `SessionGate` :

- `ensure` : session active → retour immédiat. Sinon :
  1. l'appel est mis en attente ;
  2. état `LoginRequested`, `notify_login_needed` (notification Windows) et ouverture de la fenêtre de login ;
  3. à la connexion, tous les appels en attente repartent ;
  4. à l'échéance de l'appel : `SessionError::LoginPending`. **La connexion continue** : l'appel suivant passera.
- Une seule connexion en cours par session, partagée par tous les appels qui l'attendent.
- Une seule fenêtre de login visible à la fois ; les autres sessions demandées font la queue.
- Fenêtre fermée ou saisie annulée : `SessionError::Cancelled`, retour à l'état précédent, appels en attente
  libérés avec une erreur claire.
- `mark_rejected(session, version)` : `Active` → `Expired`, secret effacé, **seulement si le secret refusé est encore
  celui de la session**. `inject` rend la version du secret injecté (`SecretVersion`, nouvelle à chaque `put` du
  store) et l'amont la rend avec le refus : un secret renouvelé entre-temps par un appel concurrent est gardé. Une
  requête partie sans secret ne marque rien.
- `connect(session)` et `disconnect(session)` pour les boutons de l'interface. `disconnect` efface le secret et
  appelle `clear_site_data` sur les domaines de la session.
- Ni tentative silencieuse, ni renouvellement proactif.

## Tests

Avec `FakeAuthUi` :

- **EX-010-01** — chaque méthode, chemin nominal ;
- **EX-010-02** — JWT mal formé, expiré, mauvaise audience, URL hors motif : ignoré ;
- **EX-010-03** — cookie attendu manquant ; contrôle Basic refusé puis accepté ; 3 échecs → abandon ;
- **EX-010-04** — N appels simultanés sur une session : une seule fenêtre ouverte, tous servis ;
- **EX-010-05** — échéance dépassée → `LoginPending`, puis connexion → l'appel suivant passe sans nouvelle fenêtre ;
- **EX-010-06** — annulation ; `mark_rejected` puis nouvelle connexion ;
- **EX-010-07** — passage à `Expired` à l'échéance (horloge de test), secret effacé, aucune fenêtre ouverte sans appel ;
- **EX-010-08** — deux sessions demandées : fenêtres l'une après l'autre ;
- **EX-010-09** — aucune fenêtre cachée n'est jamais demandée ;
- **EX-010-10** — intégration avec la passerelle : premier appel retenu puis rejoué après connexion ; 401 → reconnexion → rejeu
  unique ;
- **EX-010-11** — aucun secret dans `Debug`, les erreurs ni les événements.
- **EX-010-12** — un refus portant sur un secret déjà remplacé n'efface pas le secret plus récent.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Fenêtres WebView2 réelles (5b). Notifications réelles (5c).

## Amendements

- **2026-11-22** — Contrat `SessionGate` (spec 006) précisé pour `mark_rejected` : `inject` rend la version du
  secret injecté et `mark_rejected` la reçoit, pour ne jamais effacer un secret renouvelé par un appel concurrent
  (I007.7). Variante `SessionError::LoginFailed` ajoutée (connexion terminée sans identifiant utilisable). Critère
  ajouté : `EX-010-12`.
