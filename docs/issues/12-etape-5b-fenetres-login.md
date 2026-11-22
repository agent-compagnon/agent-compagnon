# [Étape 5b] app — fenêtres de login WebView2 (JWT, cookie, Basic), profil InPrivate

> Référence commune : `00-cadrage.md`. Dépend de : étapes 4 et 5a.

## Objectif

Implémenter `AuthUi` avec WebView2 : l'utilisateur se connecte sur la vraie page de login, l'appli récupère le
jeton ou le cookie, et rien ne touche le disque.

## À faire

### Environnement WebView2 partagé

- Créer **un seul** `ICoreWebView2Environment` (webview2-com) et le passer à chaque fenêtre de login avec
  `with_environment`. wry en crée un par WebView sinon, ce qui empêcherait le SSO d'être partagé.
- Dossier de données : `%LOCALAPPDATA%\MCP Proxy\webview\`, vidé au démarrage et à la fermeture.
- Options : conserver les arguments par défaut de wry
  (`--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`) ; `AllowSingleSignOnUsingOSPrimaryAccount`
  selon `webview.os_primary_account_sso` (faux par défaut).

### Profil

- `profile = "in_memory"` (défaut) : toutes les fenêtres en `incognito(true)`. Une **WebView gardienne** cachée,
  sur `about:blank`, est créée à la première connexion et vit jusqu'à la fermeture de l'appli ou « Tout
  déconnecter ». Elle ne navigue jamais et ne fait aucune connexion : elle maintient seulement le profil.
- Vérifier à l'exécution que le mode InPrivate est effectif. wry ignore `incognito` en silence sur un runtime trop
  ancien. S'il ne l'est pas : **échec fermé** avec un message clair, jamais de repli implicite sur un profil disque.
- `profile = "ephemeral_disk"` : profil normal dans le dossier vidé. Uniquement si la config le demande.

### Fenêtre de login

- Toujours visible, titrée « Connexion — SSO PROD », au premier plan quand c'est permis.
- `on_navigation` : seuls les hôtes de `allowed_navigation` sont autorisés. Toute autre navigation est bloquée et
  journalisée.
- Nouvelles fenêtres et téléchargements refusés. Outils de développement désactivés. Aucun script d'initialisation.
  Aucune capacité IPC.
- Fermeture par l'utilisateur → `LoginEvent::Closed`.

### Capture du JWT

- `on_web_resource_request` de Tauri ne voit pas les pages distantes : ne pas l'utiliser.
- Après création : `with_webview` → `controller()` → `CoreWebView2()` → `ICoreWebView2_22` →
  `AddWebResourceRequestedFilterWithRequestSourceKinds("*", CONTEXT_ALL, SOURCE_KINDS_ALL)`, pour voir aussi les
  iframes et les workers. Repli : `AddWebResourceRequestedFilter`. Puis `add_WebResourceRequested`.
- Le gestionnaire lit l'URL et les en-têtes, **ne modifie jamais la requête**, et n'envoie un
  `LoginEvent::HeaderCaptured` que si l'URL, l'en-tête et le schéma correspondent à la spécification.
- La valeur passe aussitôt dans un `SecretString`. Elle n'est jamais journalisée.

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
  1. en-tête `Authorization` capturé ;
  2. cookie HttpOnly lu ;
  3. mode InPrivate effectif ;
  4. SSO partagé entre deux fenêtres successives tant que la gardienne vit ;
  5. navigation hors liste bloquée ;
  6. dossier de données sans cookie persistant après fermeture.

  Cette sonde est le moyen de valider sous Windows ce qui ne peut pas l'être sous Linux. **Le point 4 n'est pas
  attesté par les sources** : s'il échoue, le signaler et basculer la config d'exemple sur `ephemeral_disk`.

## Tests (sous Linux)

- `FakeSso` : cookie posé, API appelée avec le jeton.
- Correspondance d'URL et d'en-tête, liste de navigation (hôte exact, sous-domaine, hôte voisin refusé) : logique
  dans un crate, testée.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- `cargo run -p mcp-proxy --example webview_probe` : les 6 points sont RÉUSSI.
- Un appel d'outil ouvre la fenêtre de login ; après connexion, l'appel aboutit et la carte passe à « active
  jusqu'à HH:MM ».
- Fermer la fenêtre sans se connecter renvoie une erreur claire au LLM.
- Une seconde session du même fournisseur se connecte sans ressaisie tant que l'appli tourne.
- « Déconnecter » puis « Se connecter » redemande les identifiants.

## Hors périmètre

Notifications Windows (5c).
