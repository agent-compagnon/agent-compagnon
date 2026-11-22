---
id: "012"
titre: "app — fenêtres de login WebView2 (JWT, cookie, Basic), profil InPrivate"
etape: "5b"
statut: "en cours"
depend_de: ["010", "011"]
suivi: "github#13"
---

# 012 — app — fenêtres de login WebView2 (JWT, cookie, Basic), profil InPrivate

## Objectif

Implémenter `AuthUi` avec WebView2 : l'utilisateur se connecte sur la vraie page de login, l'appli récupère le
jeton ou le cookie, et rien ne touche le disque.

## À faire

### Contrat avec la spec 010 (`mcp_proxy_auth::ui`)

- `AuthUi::login_window(LoginWindowSpec)` : la spec donne la session, le titre (« Connexion — {libellé} »),
  `login_url`, `allowed_navigation` et, pour `jwt_header` seulement, `capture()` (motif d'URL, nom d'en-tête,
  schéma) à passer au script injecté. Elle n'a aucun réglage de visibilité.
- La commande de dépôt rend un `LoginEvent::HeaderCaptured { url, header, value }` : **URL de la requête, nom de
  l'en-tête et valeur**. Le Rust de la spec 010 revérifie chaque événement (`HeaderCapture::candidate_token`, puis
  `capture::check_jwt`) et ignore sans réponse ce qui ne convient pas ; l'appli peut appeler les mêmes fonctions
  pour journaliser un refus tôt, mais ne doit jamais retenir une valeur qu'elles refusent. La valeur passe dans un
  `SecretString` dès son arrivée.
- `Navigated { url }` à chaque navigation de premier niveau acceptée ; `Closed` (ou `None`) quand l'utilisateur
  ferme la fenêtre. Une fenêtre lâchée sans `close` doit se fermer aussi.
- `cookies(url)` rend des `CookiePair { name, value: SecretString, expires }` (`None` pour un cookie de session).
- `basic_prompt` : `None` si l'utilisateur annule ou ferme ; `BasicPromptSpec::error()` est le message à afficher
  (« identifiants refusés »), `login()` l'identifiant à préremplir.
- Échec fermé du contrôle de version du runtime : `AuthUiError::RuntimeTooOld`.
- Une seule fenêtre ou saisie à la fois est demandée par l'authentificateur ; `notify_login_needed` est appelée
  juste avant l'ouverture.

### Profil WebView2 partagé, sans appel COM

- Plus d'environnement WebView2 créé à la main ni d'appel COM (`webview2-com`, `with_environment`,
  `with_webview`) : ils n'existent qu'en `unsafe` (D18).
- Chaque fenêtre de login et la fenêtre gardienne utilisent le même `data_directory(..)` :
  `%LOCALAPPDATA%\MCP Proxy\webview\`, vidé au démarrage et à la fermeture.
- Arguments du navigateur : ceux de wry par défaut (`--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`).
  Si `webview.os_primary_account_sso` est vrai (faux par défaut), `additional_browser_args` reçoit ces arguments
  plus `--enable-features=msSingleSignOnOSForPrimaryAccountIsShared`. Toutes les fenêtres reçoivent exactement les
  mêmes arguments, sans quoi WebView2 refuse de partager le dossier de données. La construction de la liste est une
  fonction pure, dans un crate.

### Profil

- `profile = "in_memory"` (défaut) : toutes les fenêtres en `incognito(true)`. Une **WebView gardienne** cachée,
  sur `about:blank`, est créée à la première connexion et vit jusqu'à la fermeture de l'appli ou « Tout
  déconnecter ». Elle ne navigue jamais et ne fait aucune connexion : elle maintient seulement le profil.
- Contrôle « InPrivate effectif » : wry ignore `incognito` en silence sur un runtime trop ancien. Avant toute
  fenêtre de login, la version du runtime lue par `tauri::webview_version()` doit être **≥ 101.0.1210.39** ;
  sinon, ou si elle est illisible : **échec fermé** avec un message clair (« runtime WebView2 trop ancien pour une
  connexion sans trace »), jamais de repli implicite sur un profil disque. La comparaison de versions est une
  fonction pure, dans un crate. Le comportement réel (aucun cookie persistant) se valide en recette.
- `profile = "ephemeral_disk"` : profil normal dans le dossier vidé. Uniquement si la config le demande.

### Fenêtre de login

- Toujours visible, titrée « Connexion — SSO PROD », au premier plan quand c'est permis.
- `on_navigation` : seuls les hôtes de `allowed_navigation` sont autorisés. Toute autre navigation est bloquée et
  journalisée.
- Nouvelles fenêtres et téléchargements refusés. Outils de développement désactivés. Un seul script
  d'initialisation, celui de la capture du JWT (méthode `jwt_header` seulement). Une seule commande accessible, le
  dépôt d'en-tête (ci-dessous) ; aucune autre capacité IPC.
- Fermeture par l'utilisateur → `LoginEvent::Closed`.

### Capture du JWT : script injecté et commande de dépôt

- `on_web_resource_request` de Tauri ne voit pas les pages distantes, et le crochet natif de WebView2
  (`add_WebResourceRequested`) n'est accessible qu'en `unsafe` : ni l'un ni l'autre n'est utilisé.
- **Script injecté** par `initialization_script_for_all_frames`, pour les fenêtres `jwt_header` seulement. Il
  enveloppe `fetch` et `XMLHttpRequest` (`open`, `setRequestHeader`, `send`) **sans jamais modifier la requête ni la
  réponse**. Pour une requête dont l'URL correspond à `capture_url_pattern`, l'en-tête à `capture_header` et le
  schéma à `capture_scheme` (le motif lui est passé à l'injection, sous forme de données JSON), il appelle la
  commande de dépôt par l'IPC de Tauri avec la seule valeur candidate, puis l'oublie. Il ne journalise rien, ne
  garde le jeton dans aucune variable globale, n'attend aucune réponse et avale toute erreur du dépôt.
- Le script a des **tests automatiques** (Vitest, fausses `fetch` et `XMLHttpRequest`), lancés par
  `cargo xtask ci` avec ceux de `ui/`. Sa source vit dans un crate et est incluse par `include_str!`.
- **Commande de dépôt** (`deposit_login_header`) : la seule exception à INV-8.
  - Capacité créée à l'exécution, à l'ouverture de chaque fenêtre de login, limitée à l'étiquette de cette fenêtre
    et aux hôtes de `allowed_navigation` (`remote`) ; retirée à sa fermeture.
  - Accepte une seule valeur candidate (chaîne, 16 Kio au plus) et l'URL de la requête ; ne renvoie **rien**, ni
    succès ni erreur, quel que soit le résultat.
  - Le Rust revérifie tout : fenêtre appelante ↔ session de login en cours, hôte de la page appelante dans
    `allowed_navigation`, motif d'URL, schéma, structure du JWT, `exp`, `aud`, `iss` (spec 010). La valeur passe
    aussitôt dans un `SecretString` ; un dépôt refusé est journalisé sans la valeur. Seul le premier jeton valide
    devient un `LoginEvent::HeaderCaptured` ; les suivants sont ignorés.
- **Limites, à connaître** :
  - les requêtes émises par un worker (service worker, worker dédié ou partagé) ne sont pas vues par le script ;
  - un script de la page peut perturber le crochet (remplacer `fetch` avant ou après lui, appeler la commande avec
    une valeur de son choix) : la page est sur un hôte autorisé, mais rien ne garantit son comportement ;
  - un cadre dont l'origine n'est pas couverte par la capacité ne peut pas déposer ;
  - c'est une source moins fiable qu'une lecture des en-têtes au niveau du navigateur. Les vérifications côté Rust
    empêchent qu'une valeur mal formée, expirée ou destinée à une autre audience soit retenue ; elles ne prouvent
    pas que le jeton a été émis par le fournisseur (la signature n'est pas vérifiée, spec 010).

### Cookies

- `cookies_for_url`, **appelé depuis une tâche asynchrone** : depuis une commande synchrone ou un gestionnaire
  d'événement, il bloque sous Windows.
- Renvoie aussi les cookies HttpOnly et Secure. Valeurs dans des `SecretString`.
- `clear_site_data(hosts)` : suppression des cookies de ces hôtes. Effacement complet par
  `clear_all_browsing_data` pour « Tout déconnecter » (5e).

### Fenêtre Basic

- Petite fenêtre locale de l'appli (page du bundle, aucun contenu distant) : login, mot de passe, « Se connecter »,
  « Annuler ».
- Commande `submit_basic_credentials`, autorisée pour cette seule fenêtre. Le mot de passe devient un
  `SecretString` dès son arrivée côté Rust ; le formulaire est vidé aussitôt.
- Le message « identifiants refusés » s'affiche dans cette fenêtre.

### Testkit et sonde

- `FakeSso` dans le testkit : serveur HTTP local avec une page de login qui pose un cookie HttpOnly, et une page
  dont le script appelle une API avec `Authorization: Bearer <jwt>`.
- `app/examples/webview_probe.rs` : ouvre une fenêtre de login sur `FakeSso` et affiche RÉUSSI ou ÉCHEC pour :
  1. en-tête `Authorization` capturé par le script injecté et déposé (par `fetch` et par `XMLHttpRequest`) ;
  2. cookie HttpOnly lu ;
  3. mode InPrivate effectif : version du runtime acceptée, et aucun cookie de la session dans le dossier de
     données pendant qu'elle est ouverte ;
  4. SSO partagé entre deux fenêtres successives tant que la gardienne vit ;
  5. navigation hors liste bloquée ;
  6. dossier de données sans cookie persistant après fermeture ;
  7. depuis la page distante, toute autre commande que le dépôt est refusée (`get_snapshot` par exemple) ;
  8. un dépôt depuis une autre fenêtre ou une valeur qui ne correspond pas (URL, schéma, JWT expiré) n'est pas
     retenu.

  Cette sonde est le moyen de valider sous Windows ce qui ne peut pas l'être sous Linux. **Les points 3 et 4 ne
  sont pas attestés par les sources** : s'ils échouent, le signaler et basculer la config d'exemple sur
  `ephemeral_disk`.

## Tests (sous Linux)

- **EX-012-01** — `FakeSso` : cookie posé, API appelée avec le jeton.
- **EX-012-02** — Correspondance d'URL et d'en-tête, liste de navigation (hôte exact, sous-domaine, hôte voisin refusé) : logique
  dans un crate, testée.
- **EX-012-08** — Script injecté (Vitest) : une requête `fetch` (en-têtes en objet, en tableau ou `Headers`, ou
  `Request`) ou `XMLHttpRequest` qui correspond donne exactement un dépôt avec la valeur ; une requête qui ne
  correspond pas (URL, en-tête, schéma) n'en donne aucun ; la requête et la réponse transmises sont identiques à
  celles d'origine ; une erreur du dépôt n'atteint pas la page ; rien n'est écrit dans la console ni gardé dans une
  variable globale.
- **EX-012-09** — Vérification du dépôt côté Rust : retenu seulement pour la fenêtre de la session en cours, depuis
  un hôte de `allowed_navigation`, avec URL, schéma, structure du JWT, `exp`, `aud` et `iss` conformes ; tout autre
  dépôt est ignoré sans réponse et journalisé sans la valeur (valeur sentinelle) ; valeur trop longue refusée ;
  seul le premier jeton valide est retenu.
- **EX-012-10** — Contrôle de version du runtime (≥ 101.0.1210.39 accepté, plus ancien, mal formé ou absent →
  échec fermé) et arguments du navigateur (arguments de wry, plus la fonction SSO seulement si
  `os_primary_account_sso` est vrai).

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- ~~**EX-012-03** (manuel) — `cargo run -p mcp-proxy --example webview_probe` : les 6 points sont RÉUSSI.~~
  (retiré le 2026-11-22 : D18, la sonde a désormais 8 points ; remplacé par `EX-012-11`)
- **EX-012-04** (manuel) — Un appel d'outil ouvre la fenêtre de login ; après connexion, l'appel aboutit et la carte passe à « active
  jusqu'à HH:MM ».
- **EX-012-05** (manuel) — Fermer la fenêtre sans se connecter renvoie une erreur claire au LLM.
- **EX-012-06** (manuel) — Une seconde session du même fournisseur se connecte sans ressaisie tant que l'appli tourne.
- **EX-012-07** (manuel) — « Déconnecter » puis « Se connecter » redemande les identifiants.
- **EX-012-11** (manuel) — `cargo run -p mcp-proxy --example webview_probe` : les 8 points sont RÉUSSI ; noter en
  particulier « InPrivate effectif » (point 3) et « SSO partagé » (point 4), non attestés par les sources.

## Hors périmètre

Notifications Windows (5c).

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur). Plus d'environnement WebView2 créé à la main ni
  d'appel COM : `data_directory(..)` et `incognito(true)` sur chaque fenêtre, SSO du compte Windows par
  `additional_browser_args`, contrôle « InPrivate effectif » par la version du runtime (`tauri::webview_version()`,
  ≥ 101.0.1210.39, sinon échec fermé). Le JWT est capturé par un script injecté
  (`initialization_script_for_all_frames`) qui appelle une commande de dépôt sans retour, seule exception à INV-8
  (constitution amendée le même jour) ; le Rust revérifie tout. Limites écrites : workers non vus, script de page
  pouvant perturber le crochet, source moins fiable qu'une lecture au niveau du navigateur. Sonde portée à 8
  points. Critère retiré : `EX-012-03` ; ajoutés : `EX-012-08` à `EX-012-11`.
- **2026-11-22** — Contrat `AuthUi` précisé par la spec 010 (section « Contrat avec la spec 010 ») :
  `LoginEvent::HeaderCaptured` porte aussi le nom de l'en-tête, la vérification côté Rust du motif, de l'en-tête,
  du schéma et du JWT est faite par la spec 010 (`mcp_proxy_auth::capture`), `AuthUiError::RuntimeTooOld` sert
  l'échec fermé du contrôle de version. Les contrôles propres à l'appli (fenêtre appelante, hôte de la page, 16 Kio)
  restent ici. Aucun critère changé.
