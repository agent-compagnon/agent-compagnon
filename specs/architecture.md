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
| Appli | Tauri 2.12 (`tray-icon`, `wry`), `windows` 0.62 (projection WinRT seulement), `winreg` 0.56 |
| HTTP/TLS | `reqwest` 0.13 en `native-tls` : SChannel et magasin de certificats Windows, aucune chaîne d'outils C |
| Secrets | `secrecy` 0.10, `zeroize` 1.9, enveloppe : `chacha20poly1305` 0.11 (XChaCha20-Poly1305), `getrandom` 0.4, clé de session enveloppée par DPAPI utilisateur (WinRT `DataProtectionProvider`) |
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
crates/gateway         mcp-proxy-gateway    registre, amont, middlewares, serveur MCP, pont
crates/deploy          mcp-proxy-deploy     installation, désinstallation, updater        (ajout)
crates/testkit         mcp-proxy-testkit    faux MCP, faux SSO, faux Artifactory          (ajout, dev)
crates/modules/*       un crate par module MCP local
app/                   mcp-proxy            appli Tauri, Windows uniquement, câblage seulement
ui/                    front Vue
config/                default.toml embarqué + catalog/*.json (instantanés d'outils)
xtask/                 cargo xtask : ci, snapshot-catalog, keygen, sign, verify
```

Répertoires à l'exécution : exe dans `%LOCALAPPDATA%\Programs\MCP Proxy\` ; données non secrètes dans
`%LOCALAPPDATA%\MCP Proxy\` (`logs\`, `cache\catalog\`, `webview\`, `update\`, `settings.json`).

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
  `RequestContext::protocol_version()`, `client_info()`, `client_capabilities()`.
- Côté client, `serve_with_lifecycle(transport, ClientLifecycleMode::Auto { .. })` sonde `server/discover` puis
  retombe sur `initialize`.
- Client HTTP streamable : pas de crochet d'en-têtes dynamiques. Il faut un type qui implémente le trait
  `StreamableHttpClient` en enveloppant `reqwest::Client` (comme `AuthClient<C>` du crate), sans oublier les
  méthodes `*_with_response_limits`. Avec son propre client, la politique de redirection est à fournir.
- Erreurs HTTP : `AuthRequired` (401 avec `WWW-Authenticate`), `InsufficientScope`, `HttpStatus`, `SessionExpired`,
  `UnexpectedServerResponse("HTTP {status}: body")`. Ce dernier **contient le corps de la réponse** : toute erreur
  passe par le filtre.
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
- Une page distante n'a accès à aucune commande tant qu'aucune capacité `remote` n'est déclarée.

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
- `tauri-winrt-notification` 0.8 : `add_button(texte, action)`, `on_activated(Fn(Option<String>))` reçoit l'action du
  bouton, `on_dismissed` donne la raison. Pas d'API pour savoir si les toasts sont désactivés ni pour retirer un
  toast : passer par le crate `windows` pour ces deux besoins.
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
