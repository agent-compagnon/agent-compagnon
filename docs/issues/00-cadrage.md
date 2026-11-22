# MCP Proxy — cadrage, architecture et plan d'implémentation

## Objet

**MCP Proxy** est une application Windows en Rust qui sert de passerelle sécurisée entre des clients LLM (Claude
Desktop, Copilot/VS Code…) et des serveurs MCP internes. Les serveurs MCP **délèguent l'authentification** à cette
application : le LLM ne voit jamais aucun secret.

Ce document est la référence commune. Chaque issue suppose qu'il a été lu. Si une issue le contredit, l'issue
l'emporte pour son périmètre ; tout autre écart se note dans `DECISIONS.md`.

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
| Appli | Tauri 2.12 (`tray-icon`, `wry`), `webview2-com` 0.39, `windows` 0.62 |
| HTTP/TLS | `reqwest` 0.13 en `native-tls` : SChannel et magasin de certificats Windows, aucune chaîne d'outils C |
| Secrets | `secrecy` 0.10, `zeroize` 1.9, `CryptProtectMemory` |
| Mises à jour | `ed25519-dalek` 3, `self-replace` 1.5, `semver` |
| Logs | `tracing`, `tracing-appender` |
| Front | Vue 3, Vite, TypeScript, Tailwind CSS, shadcn-vue (Reka UI), TanStack Table, npm |

## Architecture

```
Client LLM ──stdio──▶ mcp-proxy.exe stdio          (pont : aucun secret, aucune logique)
                          │ named pipe \\.\pipe\mcp-proxy.<SID>   (ACL : utilisateur courant)
                          ▼
                  mcp-proxy.exe                     (instance unique, icône de notification)
                  ├─ gateway  serveur MCP par connexion · registre d'outils · chaîne de middlewares
                  ├─ auth     méthodes d'authentification · authentification à la demande
                  ├─ core     sessions · secrets en mémoire · filtre de sortie · politiques · config · traces
                  ├─ UI Vue (WebView locale)  ·  fenêtres de login (WebView distante, sans IPC)
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

## Invariants de sécurité

1. Aucun secret sur disque. Aucun secret dans les logs, les erreurs, les messages renvoyés au LLM, les panics ni les
   `Debug`.
2. Un secret appartient à une seule session et ne part que vers les origines déclarées pour cette session.
3. Le LLM ne reçoit aucune donnée qui n'est pas passée par le filtre de sortie.
4. Un module local ne manipule jamais un secret : il reçoit un client HTTP déjà authentifié.
5. Sans réponse de l'utilisateur, une confirmation est refusée. En cas de doute, le code échoue fermé.
6. L'utilisateur peut durcir une règle, jamais l'assouplir.
7. Aucun remplacement de l'exe sans signature ed25519 valide, version cohérente et supérieure à la version courante.
8. Les pages distantes (login) n'ont accès à aucune commande de l'appli.
9. Pas de `unwrap()`, `expect()` ni `panic!` hors des tests. `unsafe` confiné aux modules de plateforme, avec
   commentaire `// SAFETY:`.

## Décisions validées par QCM

| # | Sujet | Décision |
|---|---|---|
| D1 | Outils connus avant toute connexion | Catalogue embarqué dans l'exe + cache local ; remplacés par la liste réelle dès qu'une session le permet |
| D2 | Périmètre MCP v1 | Outils seulement. Ressources, prompts, sampling, roots et elicitation des serveurs ne sont pas relayés |
| D3 | Volume d'outils | Exposition directe, compteur dans l'onglet MCP, alerte orange au-delà d'un seuil (100 par défaut) |
| D4 | Session expirée | Renouvellement toujours visible : notification + fenêtre de login. Ni tentative silencieuse, ni renouvellement proactif |
| D5 | Précédence des règles | Chemin exact de l'outil, puis règles de catégorie (la plus profonde d'abord), puis règles par défaut (la plus profonde d'abord) |
| D6 | Règle verrouillée | Plancher de sévérité : personne n'assouplit ; une règle plus précise ou l'utilisateur peut durcir |
| D7 | Profil WebView2 | InPrivate, en mémoire, maintenu par une WebView cachée ; repli réglable par config |
| D8 | Confirmation Windows | Toast à deux boutons ; si les arguments sont tronqués ou les toasts désactivés, fenêtre de détail au premier plan |
| D9 | Validation Windows | Contrôle croisé depuis Linux uniquement. Pas de CI GitHub |
| D10 | Runner GitLab | Inconnu : le pipeline prévoit les deux variantes (runner Windows, runner Linux + cargo-xwin) |
| D11 | Artifactory | Lecture anonyme |
| D12 | Application d'une mise à jour | Exe remplacé tout de suite, effet au prochain démarrage, bouton « Redémarrer », redémarrage automatique pour une release critique |
| D13 | Durcissements retenus | Arrêt d'urgence, réglages scellés. Écartés : exposition temporaire, saisie Basic native |
| D14 | Confort retenu | Configuration automatique des clients, désinstallation propre, canal bêta, auto-diagnostic |
| D15 | Exe de test | Cross-compilation à la demande, pas à chaque étape |
| D16 | Implémentation | Opus, issue après issue, piloté et relu dans la session de cadrage |

## Décisions prises par défaut (option la plus sûre)

| # | Décision |
|---|---|
| S1 | Un exe, trois modes : appli (instance unique), `stdio` (pont), commandes internes (`--install`, `--uninstall`, `doctor`) |
| S2 | IPC par named pipe propre à l'utilisateur : ACL sur le SID courant, clients distants rejetés, première instance exigée, binaire client vérifié. Aucun port TCP |
| S3 | Les deux générations de MCP sont gérées des deux côtés : handshake `initialize` (2025-11-25 et antérieures) et 2026-07-28 sans session |
| S4 | Budget de 50 s par appel pour les attentes utilisateur, parce que Claude Desktop coupe vers 60 s. Au-delà : erreur claire invitant à réessayer, la fenêtre de login restant ouverte |
| S5 | Canal elicitation seulement si la politique l'autorise, si le client l'annonce et s'il figure dans `elicitation_clients`. Sinon : notification Windows |
| S6 | TLS : magasin Windows, vérification jamais désactivée, aucune redirection suivie avec des identifiants |
| S7 | Origines autorisées d'une session dérivées de la config (URL des MCP et modules qui l'utilisent) |
| S8 | Sortie : la valeur est masquée et l'appel marqué dans le journal, la réponse n'est pas bloquée. Entrée : l'appel est bloqué |
| S9 | Le filtre déchiffre les variantes le temps d'une passe, puis efface. C'est le second « usage » d'un secret, avec l'injection |
| S10 | Les annotations exposées au client sont recalculées depuis la catégorie retenue, pas recopiées du serveur |
| S11 | Catégorie d'un outil : config, sinon annotations du serveur, sinon destructif. Outil inconnu : traité comme destructif, avec le plancher du profil |
| S12 | Nom exposé `{env}_{alias}_{outil}`, 64 caractères au plus, troncature + empreinte en cas de dépassement |
| S13 | Clés TOML en anglais ASCII (`allow`/`confirm`/`deny`, `read`/`write`/`destructive`) ; libellés français dans l'UI |
| S14 | UI et messages utilisateur en français ; code, commentaires et commits en anglais ; `DECISIONS.md` et issues en français |
| S15 | `app/` ne contient que du câblage Windows ; toute logique testable vit dans `crates/` et se teste sous Linux |
| S16 | Signature de mise à jour : manifeste signé (version, sha256, critique), ce qui empêche le retour à une ancienne version. Deux emplacements de clé publique. Build de release refusé avec la clé de développement |
| S17 | Fenêtres de login : navigation limitée aux domaines déclarés, aucune capacité IPC |
| S18 | « Déconnecter » efface les secrets de la session et les cookies de ses domaines |
| S19 | Pas de dump mémoire (exclusion WER), hook de panic filtré, `Debug` masqué sur les types secrets |
| S20 | Identifiants provisoires à confirmer : identifiant Tauri `com.mcpproxy.app`, AUMID `McpProxy.Desktop` |
| S21 | Aucune télémétrie : l'appli ne contacte que les MCP configurés et Artifactory |
| S22 | Messages d'erreur au LLM en français, explicites, actionnables, sans secret |

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
  (repli `AddWebResourceRequestedFilter`) et `add_WebResourceRequested`.
- `cookies_for_url(url)` renvoie aussi les cookies HttpOnly et Secure. Blocage garanti s'il est appelé depuis une
  commande synchrone ou un gestionnaire d'événement sous Windows : l'appeler depuis une tâche asynchrone.
- `incognito(true)` active `IsInPrivateModeEnabled`, mais est **ignoré en silence** si le runtime WebView2 est trop
  ancien : vérifier à l'exécution et échouer fermé.
- wry crée un `ICoreWebView2Environment` par WebView. Pour partager un profil entre fenêtres, créer l'environnement
  soi-même et le passer avec `with_environment`. C'est aussi le seul moyen de régler
  `AllowSingleSignOnUsingOSPrimaryAccount`.
- Le partage des cookies InPrivate entre fenêtres d'un même environnement n'est pas attesté par les sources : c'est
  le premier point à valider en recette (étape 5b), avec repli `ephemeral_disk`.
- Une page distante n'a accès à aucune commande tant qu'aucune capacité `remote` n'est déclarée.
- `tauri-winrt-notification` 0.8 : `add_button(texte, action)`, `on_activated(Fn(Option<String>))` reçoit l'action du
  bouton, `on_dismissed` donne la raison. Pas d'API pour savoir si les toasts sont désactivés ni pour retirer un
  toast : passer par le crate `windows` pour ces deux besoins.
- `self_replace` renomme l'exe en cours, pose le nouveau et programme la suppression de l'ancien. Le dossier de
  l'exe doit être inscriptible.

**Clients LLM** (rapports d'utilisateurs, non officiels)
- Claude Desktop coupe un appel d'outil vers 60 s, ne recharge pas fiablement la liste après `list_changed` et peut
  annoncer l'elicitation sans l'afficher.
- VS Code/Copilot plafonne à 128 outils par requête.

## Règles de travail

- Une issue à la fois, dans l'ordre. À la fin de chacune : tout compile, les vérifications passent, un court point
  est fait avant d'enchaîner.
- Vérification commune : `cargo xtask ci`, soit
  ```
  cargo fmt --all --check
  cargo clippy --all-targets -- -D warnings                 # hôte Linux, membres par défaut (sans app/)
  cargo test
  cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
  # et, dès que ui/ existe : npm ci && npm run lint && npm run typecheck && npm test && npm run build
  ```
- Ce qui ne peut pas être exécuté sous Linux (WebView2, toasts, installation, remplacement de l'exe) est livré avec
  une **recette manuelle** : liste courte de vérifications à dérouler sous Windows, ajoutée à `docs/recette.md`.
- Tout point ambigu : choisir l'option la plus sûre, la noter dans `DECISIONS.md`, continuer.
- Les API citées ci-dessus font foi dans les sources des crates, pas dans ce texte.

## Plan

Voir `README.md` de ce dossier pour la liste ordonnée des issues.
