---
id: "012"
titre: "app — fenêtres de login WebView2 (JWT, cookie, Basic), profil InPrivate"
etape: "5b"
statut: "implémentée"
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
  `%LOCALAPPDATA%\MCP Proxy\webview\`, vidé au démarrage (avant toute WebView) et à la fermeture (au mieux :
  WebView2 peut encore tenir des fichiers, le dossier est de nouveau vidé au démarrage suivant). Si le vidage du
  démarrage échoue, aucune fenêtre de login ne s'ouvre jusqu'au prochain démarrage (échec fermé) et une alerte de
  niveau erreur l'explique. Un lien symbolique à cet endroit est supprimé, jamais suivi.
- Arguments du navigateur : ceux de wry par défaut (`--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`).
  Si `webview.os_primary_account_sso` est vrai (faux par défaut), `additional_browser_args` reçoit ces arguments
  plus `--enable-features=msSingleSignOnOSForPrimaryAccountIsShared`. Toutes les fenêtres reçoivent exactement les
  mêmes arguments, sans quoi WebView2 refuse de partager le dossier de données. La construction de la liste est une
  fonction pure, dans un crate.

### Profil

- `profile = "in_memory"` (défaut) : toutes les fenêtres en `incognito(true)`. Une **WebView gardienne** cachée,
  sur `about:blank`, est créée à la première connexion et vit jusqu'à la fermeture de l'appli ou « Tout
  déconnecter ». Elle ne navigue jamais (toute navigation refusée) et ne fait aucune connexion : elle maintient
  seulement le profil, et sert à lire et supprimer les cookies du profil (`clear_site_data`). Elle existe aussi en
  `ephemeral_disk`, pour ce second usage.
- Contrôle « InPrivate effectif » : wry ignore `incognito` en silence sur un runtime trop ancien. Avant toute
  fenêtre de login, la version du runtime lue par `tauri::webview_version()` doit être **≥ 101.0.1210.39** ;
  sinon, ou si elle est illisible : **échec fermé** avec un message clair (« runtime WebView2 trop ancien pour une
  connexion sans trace »), jamais de repli implicite sur un profil disque. La comparaison de versions est une
  fonction pure, dans un crate (`mcp_proxy_auth::webview::check_runtime_version` : quatre nombres, suivis
  éventuellement du nom d'un canal, `121.0.2277.4 beta`). Le contrôle n'a lieu qu'en `in_memory`, seul profil qui
  dépend d'InPrivate. Le message s'affiche une fois par lancement (boîte de message) et chaque refus est journalisé.
  Le comportement réel (aucun cookie persistant) se valide en recette.
- `profile = "ephemeral_disk"` : profil normal dans le dossier vidé. Uniquement si la config le demande.

### Fenêtre de login

- Toujours visible, titrée « Connexion — SSO PROD », au premier plan quand c'est permis.
- `on_navigation` : seuls les hôtes de `allowed_navigation` sont autorisés, à l'identique (ni sous-domaine, ni hôte
  voisin), en `https` sur tout port, en `http` seulement vers `localhost` et `127.0.0.1` (règle des URL de la config,
  I1.14), sans identifiants dans l'URL ; tout autre schéma (`about:`, `data:`, `javascript:`, `file:`…) est refusé.
  Toute autre navigation est bloquée et journalisée avec son schéma, son hôte et son port seulement (jamais le
  chemin ni la requête). `on_navigation` ne voit que la navigation **de premier niveau** : les cadres (iframes) d'une
  page autorisée ne sont pas filtrés (voir les limites).
- Nouvelles fenêtres et téléchargements refusés. Outils de développement désactivés. Un seul script
  d'initialisation à nous, celui de la capture du JWT (méthode `jwt_header` seulement) ; Tauri injecte les siens
  (IPC, et `Object.freeze(Object.prototype)` puisque `freezePrototype` est actif pour toute l'appli). Une seule
  commande accessible, le dépôt d'en-tête (ci-dessous) ; aucune autre capacité IPC.
- `LoginEvent::Navigated` est émis à la fin du chargement de chaque page de premier niveau d'un hôte autorisé
  (`PageLoadEvent::Finished`), pour que les cookies posés par sa réponse soient lisibles.
- Fermeture par l'utilisateur → `LoginEvent::Closed`. Une fenêtre fermée par l'authentificateur, ou lâchée, est
  détruite sans événement.

### Capture du JWT : script injecté et commande de dépôt

- `on_web_resource_request` de Tauri ne voit pas les pages distantes, et le crochet natif de WebView2
  (`add_WebResourceRequested`) n'est accessible qu'en `unsafe` : ni l'un ni l'autre n'est utilisé.
- **Script injecté** par `initialization_script_for_all_frames`, pour les fenêtres `jwt_header` seulement. Il
  enveloppe `fetch` et `XMLHttpRequest` (`open`, `setRequestHeader`, `send`) **sans jamais modifier la requête ni la
  réponse**. Pour une requête dont l'URL correspond à `capture_url_pattern`, l'en-tête à `capture_header` et le
  schéma à `capture_scheme` (le motif lui est passé à l'injection, sous forme de données JSON), il appelle la
  commande de dépôt par l'IPC de Tauri avec la seule valeur candidate et l'URL de la requête
  (`{ url, value }`), puis l'oublie. Il ne journalise rien, ne garde le jeton dans aucune variable globale, n'attend
  aucune réponse et avale toute erreur du dépôt. Les en-têtes de `fetch` sont lus sous forme d'objet, de tableau de
  paires ou de `Headers`, ou sur la `Request` ; tout autre itérable est laissé de côté (le lire pourrait le consommer
  et changer la requête). Les valeurs de la config sont insérées dans le script sous forme de données
  (`mcp_proxy_auth::webview::js_data` : JSON dont tout caractère hors ASCII imprimable, et `<`, `>`, `&`, `'`,
  `` ` ``, est écrit en `\uXXXX`).
- L'IPC de Tauri (`__TAURI_INTERNALS__`) n'est injectée que dans le cadre principal : le script d'un cadre de même
  origine que la page dépose par la fenêtre principale, celui d'un cadre d'une autre origine ne peut pas déposer.
- Le script a des **tests automatiques** (Vitest, fausses `fetch` et `XMLHttpRequest`), lancés par
  `cargo xtask ci` avec ceux de `ui/`. Sa source vit dans un crate et est incluse par `include_str!`.
- **Commande de dépôt** (`deposit_login_header`) : la seule exception à INV-8.
  - Capacité créée à l'exécution (`Manager::add_capability`), à l'ouverture de chaque fenêtre `jwt_header`,
    limitée à l'étiquette de cette fenêtre, sans contexte local, et aux hôtes de `allowed_navigation` (`remote` :
    `https://{hôte}:*`, plus `http://{hôte}:*` pour `localhost` et `127.0.0.1`), avec la seule permission
    `allow-deposit-login-header`. Un hôte qui n'est pas un simple nom d'hôte, ou un motif que Tauri refuserait,
    empêche l'ouverture de la fenêtre. Tauri ne permet pas de retirer une capacité : elle est **rendue
    inopérante** à la fermeture, l'étiquette (`login-N`) n'étant jamais réutilisée et la fenêtre étant retirée des
    fenêtres de login en cours. Une fenêtre `cookie` n'a ni script ni capacité.
  - Accepte une seule valeur candidate (chaîne, 16 Kio au plus) et l'URL de la requête ; ne renvoie **rien**, ni
    succès ni erreur, quel que soit le résultat (la page reçoit toujours la même réponse vide de Tauri). Des
    arguments absents ou d'un autre type ne provoquent pas d'erreur non plus : le dépôt est ignoré. Le nom de
    l'en-tête de l'événement est celui de la config.
  - Le Rust revérifie tout : fenêtre appelante ↔ session de login en cours, hôte de la page appelante dans
    `allowed_navigation`, motif d'URL, schéma, structure du JWT, `exp`, `aud`, `iss` (spec 010). La valeur passe
    aussitôt dans un `SecretString` ; un dépôt refusé est journalisé sans la valeur ni l'URL, avec la session et la
    raison. Seul le premier jeton valide devient un `LoginEvent::HeaderCaptured` ; les suivants sont ignorés. Pour
    cela, `HeaderCapture::expectations()` donne à l'appli l'audience et l'émetteur attendus, et les vérifications
    sont celles de la spec 010 (`mcp_proxy_auth::webview::LoginWindows::deposit`), refaites ensuite par la méthode.
- **Limites, à connaître** :
  - les requêtes émises par un worker (service worker, worker dédié ou partagé) ne sont pas vues par le script ;
  - un script de la page peut perturber le crochet (remplacer `fetch` avant ou après lui, appeler la commande avec
    une valeur de son choix) : la page est sur un hôte autorisé, mais rien ne garantit son comportement ;
  - un cadre dont l'origine n'est pas celle de la page ne peut pas déposer (l'IPC de Tauri n'existe que dans le
    cadre principal) ;
  - les cadres d'une page autorisée ne sont pas filtrés par `on_navigation` (wry ne branche que la navigation de
    premier niveau) : une page autorisée peut afficher un cadre d'un autre hôte. Ce cadre ne peut ni déposer ni
    appeler une commande, et ne lit pas les cookies HttpOnly de la session ;
  - la page voit l'IPC de Tauri : elle peut appeler la commande de dépôt avec une valeur de son choix, et apprendre
    qu'une autre commande lui est refusée (le refus de Tauri), jamais la raison d'un dépôt ignoré ;
  - c'est une source moins fiable qu'une lecture des en-têtes au niveau du navigateur. Les vérifications côté Rust
    empêchent qu'une valeur mal formée, expirée ou destinée à une autre audience soit retenue ; elles ne prouvent
    pas que le jeton a été émis par le fournisseur (la signature n'est pas vérifiée, spec 010).

### Cookies

- `cookies_for_url`, **appelé depuis une tâche asynchrone** : depuis une commande synchrone ou un gestionnaire
  d'événement, il bloque sous Windows. Il est appelé dans `spawn_blocking` (il attend la boucle d'événements).
- Renvoie aussi les cookies HttpOnly et Secure. Valeurs dans des `SecretString`, expiration en `SystemTime`.
- `clear_site_data(hosts)` : suppression, par la gardienne (`cookies()` puis `delete_cookie`), des cookies du profil
  qui seraient envoyés à l'un de ces hôtes (correspondance de domaine de la RFC 6265 : un cookie d'un domaine
  parent est donc supprimé aussi). Sans gardienne (aucune connexion depuis le démarrage), il n'y a rien à
  supprimer. Effacement complet par `clear_all_browsing_data` pour « Tout déconnecter » (5e).

### Fenêtre Basic

- Petite fenêtre locale de l'appli (page du bundle, `ui/basic.html`, aucun contenu distant ; navigation limitée
  aux pages de l'appli) : login, mot de passe, « Se connecter », « Annuler ». Une fenêtre par saisie (`basic-N`),
  fermée dès la réponse ; fermer la fenêtre vaut « Annuler ». Ce qu'elle affiche (titre, identifiant prérempli,
  message, essai) lui est donné avant son chargement, sous forme de données (`window.__MCP_PROXY_BASIC__`, figé) ;
  jamais le mot de passe.
- Commande `submit_basic_credentials` (`{ answer: { login, password } }`, ou `{ answer: null }` pour « Annuler »),
  autorisée pour ces seules fenêtres (capacité statique `basic`, fenêtres `basic-*`, contexte local). Seule la
  première réponse de la fenêtre de la saisie en cours compte ; une réponse mal formée est ignorée. Le mot de passe
  devient un `SecretString` dès son arrivée côté Rust ; le formulaire est vidé avant l'envoi. La commande ne renvoie
  rien.
- Le message « identifiants refusés » s'affiche dans cette fenêtre.

### Testkit et sonde

- `FakeSso` dans le testkit : serveur HTTP local avec une page de login qui pose un cookie HttpOnly, et une page
  dont le script appelle une API avec `Authorization: Bearer <jwt>` (par `fetch` ou par `XMLHttpRequest`), plus les
  pages hostiles de la sonde (autres commandes, dépôts forgés, sortie de la liste) et `/whoami`.
- `app/examples/webview_probe.rs` : ouvre les vraies fenêtres de l'appli (`mcp_proxy::login::WebviewAuthUi`, d'où
  la bibliothèque `mcp_proxy` du paquet `app/`) sur `FakeSso`, dans son propre dossier
  (`%LOCALAPPDATA%\MCP Proxy\webview-probe\`), et affiche une ligne RÉUSSI ou ÉCHEC, avec la raison, pour :
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
  `ephemeral_disk`. Le point 3 attend 35 s fenêtre ouverte (`--attente-cookies`), WebView2 écrivant ses cookies
  en différé ; il cherche le nom du cookie, en clair dans la base de cookies (les valeurs y sont chiffrées).
  `--profil ephemeral_disk` sert de contrôle : le point 3 doit alors être en ÉCHEC, cookie trouvé.

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
- **EX-012-12** — Fenêtre Basic : le formulaire est vidé avant l'envoi, la commande reçoit l'identifiant et le mot de
  passe, « Annuler » n'envoie qu'une annulation, une seule réponse par fenêtre, le message de refus et
  l'identifiant précédent sont affichés (Vitest) ; côté Rust, seule la première réponse de la fenêtre de la saisie
  en cours est retenue, une réponse mal formée est ignorée, le mot de passe est un `SecretString`, et la fenêtre ne
  navigue que vers les pages de l'appli.
- **EX-012-13** — Capacité du dépôt : motifs `remote` des seuls hôtes de `allowed_navigation` (`https` sur tout
  port, `http` seulement pour `localhost` et `127.0.0.1`) ; un hôte qui n'est pas un simple nom d'hôte n'en donne
  aucune.
- **EX-012-14** — Dossier du profil vidé et recréé (contenu supprimé, fichier remplacé, lien symbolique supprimé sans
  être suivi).
- **EX-012-15** — Configuration des scripts : les valeurs de la config et de la saisie Basic sont écrites sous forme
  de données (ASCII imprimable sans `<`, `>`, `&`, `'`, `` ` ``, relues à l'identique), quelles qu'elles soient ;
  le script de capture porte sa configuration une seule fois.

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
- **EX-012-16** (manuel) — Fenêtre Basic : elle s'ouvre au premier plan, titrée « Connexion — {libellé} » ; un
  mauvais mot de passe la rouvre avec « identifiants refusés » et l'identifiant prérempli ; « Annuler » ou la croix
  rend une erreur claire au LLM ; le journal ne contient pas le mot de passe.
- **EX-012-17** (manuel) — Fenêtre de login : F12 et Ctrl+Maj+I n'ouvrent pas les outils de développement, le menu
  contextuel n'a pas « Inspecter » ; un lien `target=_blank` ou `window.open` n'ouvre aucune fenêtre ; un
  téléchargement est refusé ; le titre reste « Connexion — {libellé} » quelle que soit la page.
- **EX-012-18** (manuel) — `webview.os_primary_account_sso = true` (faux par défaut), sur un poste dont la session
  Windows porte un compte Entra ID ou Microsoft : la page de connexion Microsoft reconnaît ce compte sans saisie de
  l'identifiant ; avec `false`, elle le demande. L'effet de l'argument
  `msSingleSignOnOSForPrimaryAccountIsShared` en mode InPrivate n'est pas attesté par les sources : noter le
  résultat, quel qu'il soit.

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
- **2026-11-22** — Implémentation, d'après les sources de Tauri 2.12.7 et wry 0.57.3. Capacité du dépôt : Tauri
  n'a pas d'API pour retirer une capacité (`RuntimeAuthority::add_capability` ajoute seulement) ; elle est rendue
  inopérante à la fermeture (étiquette jamais réutilisée, fenêtre retirée des fenêtres en cours), créée pour les
  seules fenêtres `jwt_header`, motifs `https://{hôte}:*` (et `http://` pour la boucle locale), sans contexte local.
  L'IPC de Tauri n'existe que dans le cadre principal : un cadre de même origine dépose par la fenêtre principale,
  un cadre d'une autre origine ne peut pas déposer. `on_navigation` ne voit que la navigation de premier niveau :
  limite ajoutée pour les cadres. Navigation : hôte exact, `https` sur tout port, `http` vers la boucle locale
  seulement. `Navigated` à la fin du chargement. Contrôle de version en `in_memory` seulement ; gardienne aussi en
  `ephemeral_disk` (elle sert à `clear_site_data`) ; échec du vidage du dossier au démarrage → fenêtres de login
  refusées. Fenêtre Basic : page `basic.html`, une fenêtre par saisie, capacité statique `basic-*`. Sonde : son
  propre dossier, attente de 35 s pour le point 3, contrôle par `--profil ephemeral_disk`. Critères ajoutés :
  `EX-012-12` à `EX-012-17` ; aucun critère existant changé.
- **2026-11-22** — Critère `EX-012-18` (manuel) ajouté. Raison : l'effet réel de
  `webview.os_primary_account_sso = true` n'était vérifié nulle part, seule la construction des arguments l'est
  (`EX-012-10`). La voie documentée par WebView2 (`AllowSingleSignOnUsingOSPrimaryAccount`) n'existe qu'en `unsafe`
  (D18) ; passer par un argument du navigateur est une hypothèse, à valider sur un poste réel. Aucun critère
  existant changé.
