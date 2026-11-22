---
id: "010"
titre: "auth — méthodes d'authentification et authentification à la demande"
etape: "4"
statut: "implémentée"
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
  « jamais connectée », « active jusqu'à HH:MM », « expirée », « connexion demandée ». `label(utc_offset)` reçoit
  le décalage local, fourni par l'appli. `until` est l'échéance **marge déduite** : l'heure où la session cesse de
  servir.
- `SessionRegistry` : état de chaque session, services couverts (`SessionInfo::services`, les instances
  (environnement, MCP) qui l'utilisent), diffusion des changements (`SessionEvent { session, state, change }`,
  `SessionChange` : `LoginRequested`, `LoggedIn`, `LoginCancelled`, `LoginFailed`, `Expired`, `Rejected`,
  `Disconnected`). Seul l'authentificateur change les états.
- Une session est considérée expirée 60 s avant son échéance (`EXPIRY_MARGIN`). À l'échéance : état `Expired` et
  secret effacé, par une minuterie **et** à chaque usage (contrôle sur l'heure murale, la minuterie pouvant être en
  retard après une mise en veille). **Aucun renouvellement n'est tenté** tant qu'un appel ne le demande pas.
- Secrets versionnés : chaque `SecretStore::put` rend une `SecretVersion` nouvelle ; `wipe_version` n'efface que le
  secret de cette version.

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
    /// HeaderCaptured { url, header, value }, Navigated { url }, Closed ; None = fenêtre disparue = Closed.
    async fn next_event(&mut self) -> Option<LoginEvent>;
    async fn cookies(&self, url: &Url) -> Result<Vec<CookiePair>, AuthUiError>;
    async fn close(self: Box<Self>);   // une fenêtre lâchée sans close se ferme aussi
}
```

L'appli l'implémente avec WebView2 (5b). Le testkit fournit `FakeAuthUi`. Une fenêtre de login est **toujours
visible** : `LoginWindowSpec` (session, titre « Connexion — {libellé} », `login_url`, `allowed_navigation`, capture
d'en-tête pour `jwt_header` seulement) n'a aucun réglage de visibilité et ne se construit que dans le crate auth.

- `LoginEvent::HeaderCaptured` porte l'URL de la requête, le **nom de l'en-tête** et la valeur candidate
  (`SecretString`). `CookiePair { name, value: SecretString, expires }`. `BasicPromptSpec` : titre, identifiant de
  l'essai précédent (jamais le mot de passe), message d'erreur, numéro d'essai sur 3. `BasicInput { login, password:
  SecretString }`.
- `AuthUiError` : textes fixes (`Unavailable`, `WindowFailed`, `RuntimeTooOld`, `Cookies`, `ClearSiteData`).
- `notify_login_needed` est appelée juste avant l'ouverture de la fenêtre, quand une connexion demandée par un appel
  a son tour.

### Trait `AuthMethod` et les trois méthodes

- `JwtHeader` — capture d'un JWT dans les en-têtes des requêtes de la WebView.
  - Ne retient que les requêtes dont l'URL correspond à `capture_url_pattern`, l'en-tête à `capture_header` et le
    schéma à `capture_scheme` (`HeaderCapture::candidate_token` : 16 Kio au plus, en-tête et schéma sans casse,
    `{schéma} {jeton}`).
  - Valide la structure (3 segments base64url, en-tête JSON avec `alg`, charge utile JSON), `exp` postérieur à
    maintenant + 60 s, `nbf` s'il existe, `aud` et `iss` s'ils sont attendus (`capture::check_jwt`). La signature
    n'est pas vérifiée : ce n'est pas le rôle du proxy. Un jeton forgé de la bonne forme serait donc retenu, puis
    refusé par le serveur (401 : session expirée, nouvelle connexion) ; il ne part que vers les origines de la
    session.
  - Une valeur refusée est ignorée, journalisée avec sa raison sans la valeur ni l'URL, et la fenêtre continue
    d'attendre.
  - Échéance = `min(exp, maintenant + max_ttl_hours)`. Premier jeton valide retenu, puis fermeture.
- `Cookie` — cookie de session lu dans le gestionnaire de cookies.
  - Après une navigation correspondant à `success_url_pattern`, lit les cookies de `cookie_url` et exige tous les
    `cookie_names` ; seuls ceux-là sont gardés. Un cookie absent ou expirant dans la marge : la connexion continue
    d'attendre.
  - Échéance = `min(expiration du cookie, maintenant + ttl_hours)`.
- `Basic` — login et mot de passe saisis dans une fenêtre de l'appli.
  - Si `probe_url` est défini : requête de contrôle (`HttpProbe` : origine de la session, sans redirection, TLS
    natif). 2xx → accepté ; 401 ou 403 → « identifiants refusés », nouvelle saisie, 3 essais au plus ; autre
    statut ou erreur réseau → connexion échouée, rien n'est gardé. Une saisie invalide compte comme un essai.
  - Échéance = `maintenant + ttl_hours`, plafonnée à 24 h.
- Le trait est extensible : ajouter une méthode ne touche ni la passerelle ni le core (l'authentificateur reçoit une
  méthode par session ; `methods_from_config` construit celles de la config).
- L'injection (`Authorization: Bearer`, `Cookie`, `Authorization: Basic`) reste dans le core (2a).

### Authentification à la demande

`OnDemandAuthenticator` implémente `SessionGate` :

- `ensure` : session active → retour immédiat. Sinon :
  1. l'appel est mis en attente ;
  2. état `LoginRequested`, puis, quand c'est son tour, `notify_login_needed` (notification Windows) et ouverture
     de la fenêtre de login ;
  3. à la connexion, tous les appels en attente repartent ;
  4. à l'échéance de l'appel : `SessionError::LoginPending`. **La connexion continue** : elle tourne dans sa propre
     tâche, hors de la future de l'appel, et l'appel suivant passera.
- Une seule connexion en cours par session, partagée par tous les appels qui l'attendent.
- Une seule fenêtre de login visible à la fois ; les autres sessions demandées font la queue, dans l'ordre des
  demandes. Pas de délai maximal : la fenêtre reste ouverte jusqu'à sa fermeture par l'utilisateur.
- Fenêtre fermée ou saisie annulée : `SessionError::Cancelled`, retour à l'état précédent, appels en attente
  libérés avec une erreur claire.
- Connexion terminée sans identifiant utilisable (3 refus Basic, sonde en échec, fenêtre impossible à ouvrir) :
  `SessionError::LoginFailed`, retour à l'état précédent, détail au journal par `tracing`.
- `mark_rejected(session, version)` : `Active` → `Expired`, secret effacé, **seulement si le secret refusé est encore
  celui de la session**. `inject` rend la version du secret injecté (`SecretVersion`, nouvelle à chaque `put` du
  store) et l'amont la rend avec le refus : un secret renouvelé entre-temps par un appel concurrent est gardé. Une
  requête partie sans secret ne marque rien.
- `connect(session)` et `disconnect(session)` pour les boutons de l'interface. `connect` ne notifie pas.
  `disconnect` annule une connexion en cours (sa fenêtre se ferme, ses appels reçoivent `Cancelled`), efface le
  secret, passe à `Expired` (sauf une session jamais connectée) et appelle `clear_site_data` sur les domaines de la
  session (`allowed_navigation`, plus l'hôte de `cookie_url` ; aucun pour Basic).
- Ni tentative silencieuse, ni renouvellement proactif.

### Passerelle

- Après une connexion réussie (`SessionChange::LoggedIn`), le `ListWatcher` recharge les instances exposées et non
  coupées qui utilisent la session (`ListWatcherDeps::sessions`).
- Un échec de connexion amont dû à l'identifiant n'arme pas la temporisation de reconnexion (spec 006).
- La chaîne répond à `LoginFailed` : « connexion à « X » échouée, appel refusé. Relancez la connexion depuis MCP
  Proxy, puis réessayez. » (statut `auth_required`).

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
- **EX-010-13** — `connect` sans notification ; `disconnect` efface le secret, appelle `clear_site_data` sur les hôtes de
  la session et libère les appels en attente d'une connexion en cours, dont le résultat éventuel est ignoré.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Fenêtres WebView2 réelles (5b). Notifications réelles (5c).

## Amendements

- **2026-11-22** — Contrat `SessionGate` (spec 006) précisé pour `mark_rejected` : `inject` rend la version du
  secret injecté et `mark_rejected` la reçoit, pour ne jamais effacer un secret renouvelé par un appel concurrent
  (I007.7). Variante `SessionError::LoginFailed` ajoutée (connexion terminée sans identifiant utilisable). Critère
  ajouté : `EX-010-12`.
- **2026-11-22** — Critère ajouté : `EX-010-13` (boutons « Se connecter » et « Déconnecter »), qui n'avait pas de
  test prévu.
- **2026-11-22** — Précisions de réalisation, sans critère changé : `LoginEvent::HeaderCaptured` porte aussi le nom
  de l'en-tête (le script injecté de la spec 012 le choisit, le Rust le revérifie) ; notification au moment où la
  fenêtre s'ouvre plutôt qu'à la demande, pour qu'elle corresponde à la fenêtre affichée ; `Active { until }` porte
  l'échéance marge déduite ; `exp` doit dépasser maintenant + 60 s et `nbf` est contrôlé ; sonde Basic : seul un 2xx
  accepte, une sonde en échec arrête la connexion sans nouvelle saisie ; `disconnect` laisse la session `Expired`.
  Côté passerelle : rechargement des listes après connexion (spec 008 amendée) et pas de temporisation de
  reconnexion après un refus d'identifiant (spec 006 amendée). Raisons : `DECISIONS.md`, I010.4 à I010.12.
- **2026-11-22** — Spec 011 : un `AuthUiError::Unavailable` (aucune interface de connexion, comme l'`AuthUi`
  provisoire de l'appli avant la spec 012) termine la connexion par `SessionError::Unavailable` au lieu de
  `LoginFailed` : le LLM lit « connexion à « X » impossible pour le moment », et non une invitation à relancer une
  connexion qui ne peut pas s'ouvrir. L'état revient à ce qu'il était, l'événement reste `LoginFailed`. Prouvé par
  `EX-011-13`.
