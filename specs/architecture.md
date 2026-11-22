# Architecture de MCP Proxy

**MCP Proxy** est une application Windows en Rust qui sert de passerelle sécurisée entre des clients LLM (Claude
Desktop, Copilot/VS Code…) et des serveurs MCP internes. Les serveurs MCP **délèguent l'authentification** à cette
application : le LLM ne voit jamais aucun secret.

Ce document décrit le cadre commun à toutes les specs. Les invariants et les règles sont dans `constitution.md`,
les décisions et leurs raisons dans `DECISIONS.md`.

## Contraintes

- Pas d'app ID ni d'URL de redirection : **pas d'OAuth/PKCE**. L'utilisateur se connecte sur la vraie page de login
  dans une WebView et l'appli récupère ce dont elle a besoin.
- 8 environnements (DEV, INT, QUA, REC, PERF, PPROD, PROD, PRA), 3 à 6 MCP chacun.
- Jetons à durée de vie courte (1 jour au plus).
- Déploiement : **un seul `.exe`**, sans droits admin, mis à jour automatiquement depuis Artifactory. CI : GitLab.
- Le développement se fait dans un conteneur Linux. Le code Windows y est **contrôlé à la compilation** (cible
  `x86_64-pc-windows-msvc`) mais jamais exécuté : la recette sous Windows est manuelle.

## Stack (versions vérifiées sur crates.io le 2026-11-22)

| Domaine | Choix |
|---|---|
| Rust | 1.97.0 épinglé, édition 2024, workspace Cargo |
| MCP | `rmcp` 3.11 — features `server`, `client`, `transport-io`, `transport-streamable-http-client-reqwest`, `reqwest-native-tls`, `elicitation`, `request-state` ; `transport-streamable-http-server` pour les tests |
| Appli | Tauri 2.12 (`tray-icon`, `wry`), `tauri-plugin-single-instance` 2.5, `windows` 0.62 (projection WinRT seulement), `winreg` 0.56, `rfd` 0.18 (boîtes de message) |
| HTTP/TLS | `reqwest` 0.13 en `native-tls` : SChannel et magasin de certificats Windows, aucune chaîne d'outils C |
| Secrets | `secrecy` 0.10, `zeroize` 1.9, enveloppe : `chacha20poly1305` 0.11 (XChaCha20-Poly1305), `getrandom` 0.4, clé de session enveloppée par DPAPI utilisateur (WinRT `DataProtectionProvider`) |
| Réglages scellés | `hmac` 0.13 (HMAC-SHA256 avec `sha2` 0.11), clé enveloppée par DPAPI utilisateur (spec 014) |
| Mises à jour | `ed25519-dalek` 3, `self-replace` 1.5, `semver` |
| Filtre de sortie | `memchr` 2.8, `regex` 1.13 (motifs génériques seulement), `serde_json` 1.0 |
| Logs | `tracing`, `tracing-subscriber` 0.3, `tracing-appender` |
| Front | Vue 3, Vite, TypeScript, Tailwind CSS, shadcn-vue (Reka UI), TanStack Table, npm |

## Architecture

```
Client LLM ──stdio──▶ mcp-proxy.exe stdio          (pont : aucun secret, aucune logique)
                          │ named pipe \\.\pipe\mcp-proxy.<utilisateur>   (tokio, première instance, clients
                          │   distants rejetés ; preuve mutuelle DPAPI avant tout octet MCP)
                          ▼
                  mcp-proxy.exe                     (instance unique, icône de notification)
                  ├─ gateway  serveur MCP par connexion · registre d'outils · chaîne de middlewares
                  ├─ auth     méthodes d'authentification · authentification à la demande
                  ├─ core     sessions · secrets en mémoire · filtre de sortie · politiques · config · traces
                  ├─ UI Vue (WebView locale)  ·  fenêtres de login (WebView distante, une seule commande : dépôt d'en-tête)
                  └─ amont    client MCP HTTP streamable par (environnement, MCP)  ·  modules locaux
```

```
crates/core            mcp-proxy-core       config, sessions, secrets, filtre, politiques, réglages, traces
crates/auth            mcp-proxy-auth       trait AuthMethod + implémentations, authentification à la demande
crates/gateway         mcp-proxy-gateway    registre, amont, middlewares, serveur MCP, pont, vue, assemblage
crates/deploy          mcp-proxy-deploy     installation, désinstallation, updater        (ajout)
crates/testkit         mcp-proxy-testkit    faux MCP, faux SSO, faux Artifactory          (ajout, dev)
crates/modules/*       un crate par module MCP local
app/                   mcp-proxy            appli Tauri, Windows uniquement, câblage seulement
ui/                    front Vue
config/                default.toml embarqué + catalog/*.json (instantanés d'outils)
xtask/                 cargo xtask : ci, snapshot-catalog, keygen, sign, verify
```

Répertoires à l'exécution : exe dans `%LOCALAPPDATA%\Programs\MCP Proxy\` ; données non secrètes dans
`%LOCALAPPDATA%\MCP Proxy\` (`logs\`, `cache\catalog\`, `webview\`, `update\`, `settings.json`, `settings.key` : clé
du scellement des réglages et du cache, enveloppée par DPAPI, spec 014).

### Parcours d'un `tools/call`

1. Résolution du nom exposé vers (environnement, MCP, outil). Refus si l'environnement n'est pas exposé, si le MCP
   est coupé ou si la passerelle est suspendue.
2. Politique : `deny` donne un refus immédiat.
3. Scan des arguments : un secret connu bloque l'appel.
4. Limite de débit du MCP.
5. Confirmation si la politique l'exige. Sans réponse dans le délai, c'est un refus.
6. Session : connexion à la demande, attente bornée.
7. Appel amont : identifiants injectés dans les en-têtes au moment de l'envoi.
8. Retour : filtre de sortie sur toute la réponse, erreurs comprises.
9. Trace : span, puis enregistrement déjà filtré dans le tampon circulaire.

## Faits techniques vérifiés dans les sources (à revérifier au moment de coder)

**rmcp 3.11**
- Versions gérées : 2024-11-05 à 2025-11-25 (handshake `initialize`) et 2026-07-28 (sans session, version et
  capacités du client dans le `_meta` de chaque requête, `server/discover`).
- Côté serveur, `serve` accepte les deux générations sur le même transport. Identité du client par requête :
  `RequestContext::protocol_version()`, `client_info()`, `client_capabilities()`. **Vérifié à l'exécution** :
  `serve` ne rend la main qu'après `initialize` ou, pour un client 2026-07-28, après sa première requête autre que
  `server/discover` (il répond aux `server/discover` en attendant) : ne pas l'attendre avant de laisser le client
  avancer.
- Côté client, `serve_with_lifecycle(transport, ClientLifecycleMode::Auto { .. })` sonde `server/discover` puis
  retombe sur `initialize` **si le serveur ne connaît pas la méthode** (`-32601`, 4xx hors 401/403, ou pas de réponse
  en 10 s). Un serveur rmcp dont `supported_protocol_versions` exclut 2026-07-28 répond au contraire par la liste de
  ses versions, et le client échoue (« no compatible protocol version ») au lieu de se replier.
- Le client attache un `progressToken` à chaque requête (`Peer::send_request_with_option`) ; il n'est connu qu'une
  fois la requête mise en file. `Peer::list_tools` met les listes en cache selon `ttl_ms`, cache invalidé par
  `list_changed`, et sert par défaut une liste périmée si la requête échoue : la passerelle désactive ce cache
  (spec 008).
- Client HTTP streamable : pas de crochet d'en-têtes dynamiques. Il faut un type qui implémente le trait
  `StreamableHttpClient` en enveloppant `reqwest::Client` (comme `AuthClient<C>` du crate). Les limites passent par
  `post_message_with_response_limits` et les deux `*_with_max_sse_event_size` (il n'existe pas de
  `get_stream_with_response_limits`). **Vérifié à la compilation** : l'implémentation du SDK pour `reqwest::Client`
  peut être réutilisée en lui passant l'en-tête d'authentification dans `custom_headers` (seuls `Accept`,
  `Mcp-Session-Id`, `Last-Event-Id` y sont réservés), ce qui garde ses limites et son analyse SSE. Le type `Sse` des
  flux vient du crate `sse-stream` 0.2, non réexporté. Avec son propre client, la politique de redirection est à
  fournir (le client par défaut suit les 307/308 de même origine).
- Erreurs HTTP : `AuthRequired` (401 **avec** `WWW-Authenticate`), `InsufficientScope` (403 avec défi), `HttpStatus`
  (429 et 5xx, sans corps), `SessionExpired`, `UnexpectedServerResponse("HTTP {status}: body")`. Ce dernier
  **contient le corps de la réponse** : toute erreur passe par le filtre. Un 401 ou 403 **sans** défi arrive sous
  cette dernière forme (`HTTP 401 Unauthorized: …`) : aucune variante ne le distingue. Les erreurs de requête
  arrivent en `ServiceError::TransportSend` sans source exploitable pour retrouver le statut.
- Elicitation : `peer.create_elicitation(..)` en ancien protocole seulement. En 2026-07-28, `call_tool` renvoie
  `CallToolResponse::InputRequired(InputRequiredResult { input_requests, request_state, .. })` et le client rejoue
  l'appel avec `input_responses` + `request_state`. `RequestStateCodec` (feature `request-state`) signe l'état
  (HMAC-SHA256, données associées, durée de vie) sans le chiffrer. Aucun utilitaire ne couvre les deux générations :
  le handler branche sur `context.protocol_version()`.
- Changement de liste : `notifications/tools/list_changed` existe toujours. En 2026-07-28 il passe par
  `subscriptions/listen` (`Peer::listen(SubscriptionFilter { tools_list_changed, .. })` côté client,
  `ServerHandler::listen` + `SubscriptionSink::notify_tool_list_changed` côté serveur) et les listes portent
  `ttl_ms`. En ancien protocole : `ClientHandler::on_tool_list_changed`, `Peer::notify_tool_list_changed`.
- Nom d'outil : 1 à 128 caractères `[A-Za-z0-9._-]`, simple avertissement.
- Délais : `PeerRequestOptions { timeout, reset_timeout_on_progress, max_total_timeout }`, aucun délai par défaut.
  `notify_progress` fonctionne dans les deux générations.

**Tauri 2.12 / wry 0.57 / WebView2**
- `on_web_resource_request` ne voit que le protocole `tauri://`, pas les pages distantes. Pour capturer des
  en-têtes : `with_webview` → `controller()`, puis `ICoreWebView2_22::AddWebResourceRequestedFilterWithRequestSourceKinds`
  (repli `AddWebResourceRequestedFilter`) et `add_WebResourceRequested`. **Toujours vrai, mais plus utilisé** :
  ces appels COM n'existent qu'en `unsafe` (D18). Le JWT est capturé par un script injecté (spec 012).
- `cookies_for_url(url)` renvoie aussi les cookies HttpOnly et Secure. Blocage garanti s'il est appelé depuis une
  commande synchrone ou un gestionnaire d'événement sous Windows : l'appeler depuis une tâche asynchrone.
- `incognito(true)` active `IsInPrivateModeEnabled`, mais est **ignoré en silence** si le runtime WebView2 est trop
  ancien : vérifier à l'exécution et échouer fermé.
- wry crée un `ICoreWebView2Environment` par WebView. Pour partager un profil entre fenêtres, créer l'environnement
  soi-même et le passer avec `with_environment`. C'est aussi le seul moyen de régler
  `AllowSingleSignOnUsingOSPrimaryAccount`. **Toujours vrai, mais plus utilisé** (création COM, donc `unsafe`, D18) :
  chaque fenêtre de login reçoit le même `data_directory(..)` et `incognito(true)`, et le SSO du compte Windows passe
  par `additional_browser_args` (spec 012).
- Le partage des cookies InPrivate entre fenêtres d'un même environnement n'est pas attesté par les sources : c'est
  le premier point à valider en recette (étape 5b), avec repli `ephemeral_disk`.
- Une page distante n'a accès à aucune commande tant qu'aucune capacité `remote` ne la couvre : pour une origine
  distante, Tauri vérifie l'ACL de toute commande, sauf sa commande interne de lecture des canaux
  (`FETCH_CHANNEL_DATA_COMMAND`), exemptée ; l'appli n'utilise aucun `Channel`, elle n'a donc rien à lire.
- **Vérifié dans les sources** (spec 012, compilé en cible Windows, non exécuté) :
  - `Manager::add_capability(CapabilityBuilder)` (feature `dynamic-acl`, active par défaut) **ajoute** une capacité
    à l'exécution ; aucune API ne la retire. La résolution se termine par un `unwrap` : une permission inconnue ou
    un motif `remote` invalide fait paniquer Tauri, d'où le contrôle préalable par `RemoteUrlPattern::from_str`.
  - Motifs `remote` (`urlpattern`) : chemin, requête et fragment sont des jokers ; `https://hôte` ne couvre que le
    port par défaut, `https://hôte:*` tous les ports (vérifié par un essai d'`urlpattern` 0.6). Le motif est
    comparé à l'en-tête `Origin` de la requête IPC (protocole `ipc://`) ou à l'URL de la page (repli
    `postMessage`).
  - L'IPC (`window.__TAURI_INTERNALS__`, `invoke` non modifiable, clé d'invocation) n'est injectée que dans le
    cadre principal ; `initialization_script_for_all_frames` s'exécute dans tous les cadres, après les scripts de
    Tauri dans le cadre principal. `freezePrototype` est injecté dans le cadre principal de toute WebView, pages
    distantes comprises.
  - Une commande `async` qui ne rend rien répond toujours `null` à la page.
  - wry : `on_navigation` est branché sur `NavigationStarting`, navigation de premier niveau seulement (pas de
    `FrameNavigationStarting`) ; une demande de nouvelle fenêtre refusée par `on_new_window` est marquée traitée
    sans fenêtre ; `on_download` qui rend `false` annule le téléchargement ; les outils de développement sont
    actifs par défaut en compilation de débogage (`devtools(false)` explicite) ; `incognito` passe par
    `ICoreWebView2ControllerOptions::SetIsInPrivateModeEnabled`. Tauri garde un contexte par `data_directory`,
    wry crée un environnement par WebView avec `additional_browser_args` (par défaut
    `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`).
  - `cookies_for_url` envoie un message à la boucle d'événements et attend sa réponse : appelé dans
    `spawn_blocking`, jamais depuis le fil principal. `cookies()` et `delete_cookie` de même.

**API sûres vérifiées** (compilation en cible Windows sous `#![forbid(unsafe_code)]` et clippy `-D warnings`, le
2026-11-22 ; rien n'a été exécuté, le comportement réel se valide en recette)
- DPAPI utilisateur par WinRT (`windows` 0.62, features `Foundation`, `Security_Cryptography`,
  `Security_Cryptography_DataProtection`, `Storage_Streams`) :
  `DataProtectionProvider::CreateOverloadExplicit(&HSTRING::from("LOCAL=user"))?.ProtectAsync(&buffer)?.join()?`,
  `DataProtectionProvider::new()?.UnprotectAsync(&buffer)?.join()?` ; tampons par
  `CryptographicBuffer::CreateFromByteArray(&[u8])` et `CryptographicBuffer::CopyToByteArray(&buffer, &mut
  Array::<u8>::new())`. Les `IBuffer` intermédiaires ne peuvent pas être effacés sans `unsafe`.
- État des toasts : `ToastNotificationManager::CreateToastNotifierWithId(..)?.Setting()?` (feature
  `UI_Notifications`).
- Named pipe : `tokio::net::windows::named_pipe::ServerOptions::new().first_pipe_instance(true)
  .reject_remote_clients(true).create(name)` (descripteur de sécurité par défaut).
- Exclusion des rapports d'erreur Windows : valeur DWORD `1` nommée d'après l'exe sous
  `HKCU\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications`, avec `winreg` 0.56.
- Hors de portée sans `unsafe` : tout appel Win32 ou COM direct, donc `CryptProtectMemory`, `CryptProtectData`,
  `GetNamedPipeClientProcessId`, `SetCurrentProcessExplicitAppUserModelID`, `AttachConsole`, `IShellLinkW`, et toute
  l'API COM de WebView2 (`with_webview`, `add_WebResourceRequested`, création d'un `ICoreWebView2Environment`).
- `tauri-winrt-notification` 0.8.1 (lu dans les sources, non utilisé) : `add_button(texte, action)`,
  `on_activated(Fn(Option<String>))` reçoit les arguments du bouton (ou du toast), `on_dismissed` la raison. Mais
  `Toast::show` crée le `ToastNotification` en interne sans le rendre, ne pose d'étiquette qu'avec une barre de
  progression, ne relaie pas `Failed` et ne règle pas d'échéance : on ne peut ni retirer le toast ni savoir qu'il a
  échoué. Son exemple `unpackaged_app` utilise un AUMID inscrit au registre seulement (`DisplayName`, `IconUri` vers
  un PNG).
- Toasts par la projection WinRT (`windows` 0.62, features `Foundation`, `Data_Xml_Dom`, `UI_Notifications`,
  compilé en cible Windows, non exécuté) : `XmlDocument` (`CreateElement`, `SetAttribute`, `SetInnerText`,
  `AppendChild`), `ToastNotification::CreateToastNotification`, `SetTag`, `SetGroup`, `SetExpiresOnReboot`,
  `SetExpirationTime` (`PropertyValue::CreateDateTime(..)?.cast::<IReference<DateTime>>()`), événements
  `Activated` (arguments par `cast::<ToastActivatedEventArgs>()`), `Dismissed` (`ToastDismissalReason`), `Failed` ;
  `CreateToastNotifierWithId(aumid)?.Show/Hide/Setting` ; `ToastNotificationManager::History()?`
  (`RemoveGroupedTagWithId`, `ClearWithId`) ; « Ne pas déranger » par
  `ToastNotificationManager::GetDefault()?.NotificationMode()` (`Unrestricted`, `PriorityOnly`, `AlarmsOnly`), que
  `Setting` ne reflète pas. `ToastNotification`, `ToastNotifier` sont `Send + Sync`. Non attesté par les sources :
  que les clics soient remis au processus quand l'AUMID n'est inscrit qu'au registre, sans activateur COM ni
  raccourci (recette `EX-013-05`, `EX-013-10`).
- `self_replace` renomme l'exe en cours, pose le nouveau et programme la suppression de l'ancien. Le dossier de
  l'exe doit être inscriptible.

**Clients LLM** (rapports d'utilisateurs, non officiels)
- Claude Desktop coupe un appel d'outil vers 60 s, ne recharge pas fiablement la liste après `list_changed` et peut
  annoncer l'elicitation sans l'afficher.
- VS Code/Copilot plafonne à 128 outils par requête.

## Amendements

- **2026-11-22** — Aucun `unsafe` dans le dépôt (D18). Ligne « Secrets » : enveloppe AEAD + DPAPI WinRT au lieu de
  `CryptProtectMemory` ; ligne « Appli » : plus de `webview2-com`, ajout de `winreg`. Schéma : le pipe n'a plus
  d'ACL sur mesure (nom par utilisateur, première instance, clients distants rejetés, preuve mutuelle DPAPI) et les
  fenêtres de login ont une seule commande, le dépôt d'en-tête (exception à INV-8). Faits techniques : les faits sur
  le crochet natif et l'environnement WebView2 restent vrais mais ne sont plus utilisés ; ajout des API sûres
  vérifiées.
- **2026-11-22** — Faits sur rmcp 3.11 corrigés après compilation et tests de la spec 006 : `serve` côté serveur ne
  rend la main qu'à la première requête d'un client 2026-07-28 ; repli d'`Auto` conditionné à un refus de
  `server/discover` ; méthodes de limites réelles du trait `StreamableHttpClient` (pas de
  `get_stream_with_response_limits`), réutilisation de l'implémentation reqwest du SDK par `custom_headers`, type
  `Sse` du crate `sse-stream` ; 401/403 sans défi en `UnexpectedServerResponse` ; jeton de progression et cache des
  listes du client. Les features `elicitation` et `request-state` ne sont pas encore activées (spec 007).
- **2026-11-22** — Features `elicitation` et `request-state` de rmcp activées (spec 007). Vérifié à l'exécution :
  `RunningService::call_tool` côté client pilote seul les tours `input_required` par le `ClientHandler`
  (`create_elicitation`), `call_tool_once` rend l'`InputRequiredResult` brut ; `RequestStateCodec::seal_json_with` /
  `open_json_with` (données associées, durée de vie) ; `Peer::create_elicitation_with_timeout` en ancien protocole.
  Un 401 sur une session ancien protocole peut aussi refermer la connexion : le rejeu peut alors être refusé dès la
  reconnexion, avant son `tools/call`.
- **2026-11-22** — Faits sur rmcp 3.11 constatés dans les sources pour la spec 008 : le cache de réponses du client
  (`ClientCacheConfig`) est actif par défaut et sert une liste périmée en cas d'erreur (`serve_stale_on_error`) ; il
  se désactive par `Peer::set_response_cache_config(ClientCacheConfig::disabled())`. Un serveur 2026-07-28 qui ne
  donne pas `ttl_ms` envoie `ttl_ms = 0` (`fill_missing_cache_hints`). Les notifications d'un abonnement
  `subscriptions/listen` ne passent pas par le `ClientHandler` ; la fin d'un abonnement est donnée par
  `Subscription::end` (`Abrupt` : à rouvrir). Le transport HTTP client réinitialise en silence une session expirée
  (404, `reinit_on_expired_session`, actif par défaut). Côté serveur, une session ancien protocole expire après 300 s
  d'inactivité (`SessionConfig::DEFAULT_KEEP_ALIVE`).
- **2026-11-22** — Modules locaux (spec 025) : contrat dans `mcp_proxy_core::module`, registre des modules compilés
  dans `mcp_proxy_gateway::modules` (la passerelle dépend de `crates/modules/*`, le core d'aucun), `LocalUpstream` à
  côté de `HttpUpstream`. Le client authentifié des modules injecte par le `SessionGate` et ne joint que l'origine
  de `base_url`.
- **2026-11-22** — Spec 011 : l'assemblage de la passerelle vit dans `mcp_proxy_gateway::runtime` (testé sous
  Linux), la vue de l'interface dans `gateway::view`, le nom du pipe et la preuve mutuelle dans `gateway::bridge` ;
  le crate `gateway` dépend du crate `auth`. Ajout de `tauri-plugin-single-instance` et de `rfd` (boîtes de message
  sans appel Win32 direct). Fait constaté : le contrôle Windows depuis Linux ne compile pas les ressources de l'exe
  (icône, manifeste), ce qui ne se vérifie que par une compilation sous Windows.
- **2026-11-22** — Spec 012 : faits sur Tauri 2.12.7 et wry 0.57.3 constatés dans les sources (capacités ajoutées à
  l'exécution sans retrait possible et panique sur une résolution invalide, motifs `remote` et ports, IPC dans le
  cadre principal seulement, `freezePrototype` dans les pages distantes, commande interne des canaux exemptée de
  l'ACL, `on_navigation` limité au premier niveau, appels de cookies bloquants). Ce qui décide des fenêtres de
  login vit dans `mcp_proxy_auth::webview` ; `app/` devient bibliothèque `mcp_proxy` et binaire, pour la sonde.
- **2026-11-22** — Spec 013 : faits sur les toasts corrigés après lecture des sources de `tauri-winrt-notification`
  0.8.1 et du crate `windows` 0.62 : ce crate ne permet ni de retirer un toast ni d'en suivre l'échec, l'appli passe
  donc directement par la projection WinRT ; « Ne pas déranger » se lit dans `NotificationMode`, pas dans `Setting`.
  Les notifications et confirmations sont décidées dans `mcp_proxy_gateway::notify`.
- **2026-11-22** — Spec 014 : `settings.key` ajouté aux données locales ; dépendance `hmac` 0.13 (déjà tirée par
  rmcp). Le scellement vit dans `mcp_proxy_core::sealed`.
- **2026-11-22** — Spec 015 : constaté dans les sources de Tauri 2.12.7 et wry 0.57.3, `clear_all_browsing_data`
  passe par `ICoreWebView2Profile2::ClearBrowsingDataAll` sans attendre sa fin ni rendre d'erreur (elle n'est que
  journalisée par la boucle d'événements). Les gestes d'urgence vivent dans `mcp_proxy_gateway::control`.
