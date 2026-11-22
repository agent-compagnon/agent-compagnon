---
id: "011"
titre: "app — squelette Tauri, pont stdio (named pipe), instance unique, tray, réglages"
etape: "5a"
statut: "implémentée"
depend_de: ["001", "002", "003", "004", "005", "006", "007", "008", "009", "010"]
suivi: "github#12"
---

# 011 — app — squelette Tauri, pont stdio (named pipe), instance unique, tray, réglages

## Objectif

Avoir un exe qui démarre dans la zone de notification, héberge la passerelle, et auquel un client LLM se connecte
en lançant le même exe en mode `stdio`.

## À faire

### Crate `app/` (paquet et binaire `mcp-proxy`)

- Windows uniquement : `compile_error!` sur les autres systèmes. Ajouté au workspace, exclu de `default-members`.
- `#![windows_subsystem = "windows"]` hors debug. Le mode `stdio` fonctionne avec les poignées héritées du client.
- Aucun `unsafe` dans `app/`, comme dans tout le dépôt (`forbid` au niveau du workspace, D18). Une API Windows
  qui n'existe qu'en `unsafe` se remplace par une API sûre (WinRT du crate `windows`, tokio, Tauri, `winreg`) ou ne
  s'utilise pas.
- **Câblage seulement.** Toute logique testable vit dans `crates/` : si un morceau de cette issue peut être testé
  sous Linux, il va dans un crate. Réalisation : `app/src/` ne contient que `main.rs` (choix du mode), `app.rs`
  (Tauri, icône, fenêtre), `pipe.rs` (pipe tokio), `bridge.rs` (mode `stdio`), `dpapi.rs` (`PeerProof` par DPAPI),
  `dialog.rs` (boîtes de message) et `wer.rs` (registre). Le reste vit dans `mcp_proxy_gateway` : `launch`
  (arguments), `bridge` (nom du pipe, preuve, relais), `view` (`AppSnapshot`, `AppStatus`) et `runtime` (assemblage,
  `Gateway`, échecs de démarrage) ; le crate `gateway` dépend désormais du crate `auth`.

### Modes de lancement

| Commande | Rôle |
|---|---|
| `mcp-proxy.exe` | Appli : instance unique, icône de notification, passerelle |
| `mcp-proxy.exe stdio` | Pont : relaie stdin/stdout vers l'appli. Aucun secret, aucune logique |

Les commandes internes (`--install`, `--uninstall`, `doctor`) arrivent aux étapes 7. Analyse stricte
(`gateway::launch::parse_launch`) : aucun argument ou exactement `stdio` ; tout autre argument, un argument non
Unicode ou plus d'un argument sont refusés (boîte de message, message sur stderr, code de sortie 2).

### Tauri

- `tauri.conf.json` : `productName = "MCP Proxy"`, identifiant `com.mcpproxy.app`, `frontendDist = ../ui/dist`,
  bundle désactivé, aucune fenêtre déclarée (elles sont créées à l'exécution), CSP stricte.
  CSP : `default-src 'self'`, `script-src 'self'`, `style-src 'self'`, `img-src 'self' data:`, `font-src 'self'`,
  `connect-src ipc: http://ipc.localhost`, `object-src`, `base-uri`, `form-action` et `frame-ancestors` à
  `'none'` ; `freezePrototype`, pas de `withGlobalTauri`.
- Capacités : la fenêtre principale n'a que les commandes dont elle a besoin. Aucune capacité `remote`.
  Réalisation : `get_snapshot` est déclarée dans le manifeste de l'appli (`build.rs`), donc refusée à toute fenêtre
  qui n'en a pas la permission ; la capacité `main` (fenêtre `main` seulement) donne `allow-get-snapshot`,
  `core:event:allow-listen` et `core:event:allow-unlisten`, rien d'autre.
- `tauri-plugin-single-instance` : un second lancement met la fenêtre principale au premier plan.
- Fenêtre principale créée cachée. Elle s'ouvre au clic sur l'icône, par le menu « Ouvrir », au premier lancement
  (choix du profil) ou au second lancement. La fermer la cache ; l'appli continue. Seul « Quitter » arrête l'appli
  (le journal est vidé à l'arrêt).
- `ui/` : squelette Vite + Vue 3 + TypeScript avec une page vide, pour que `ui/dist` existe. `cargo xtask ci`
  construit `ui/` avant le contrôle Windows. Scripts réels : `lint` (eslint, règles typées strictes de
  typescript-eslint et de eslint-plugin-vue, aucun avertissement toléré), `typecheck` (vue-tsc), `test` (Vitest,
  jsdom) et `build` (Vite, chemins relatifs, rien d'inline). TypeScript 6.0 : typescript-eslint n'accepte pas 7.

### Icône de notification

- Trois états : neutre, attention (orange : une session ou une confirmation attend), erreur.
- Le calcul de l'état (`AppStatus`) est une fonction pure, dans un crate, testée sous Linux.
- Menu : « Ouvrir », « Quitter ». Infobulle avec l'état. Trois jeux d'icônes lisibles sur barre claire et sombre.
- Réalisation : `gateway::view::AppStatus::of` ; l'attention passe avant l'erreur (un appel qui attend a une
  échéance, une erreur reste affichée dans la fenêtre). Une session attend quand son état est « connexion
  demandée » ; une erreur est une alerte de niveau erreur de l'`AppSnapshot` (aujourd'hui : réglages illisibles
  remplacés par les plus stricts). Infobulle : « MCP Proxy — prêt », ou le décompte, par exemple « MCP Proxy —
  1 connexion attend, 2 erreurs ». Icônes : disque sombre à point blanc (neutre), disque orange à point sombre
  (attention), disque rouge à croix blanche (erreur), tous cerclés de blanc ; dessinées par `cargo xtask icons`
  dans `app/icons/`, avec l'icône de l'appli (`icon.ico`, 16 à 256 px) ; un test de `xtask` vérifie que les
  fichiers du dépôt sont à jour. Clic gauche : ouvre la fenêtre ; clic droit : menu.

### Named pipe

- Nom : `\\.\pipe\mcp-proxy.<utilisateur>`, où `<utilisateur>` est le domaine et le nom de l'utilisateur
  Windows, réduits à `[a-z0-9._-]`. Une collision de noms ne peut qu'empêcher le démarrage ou la connexion (voir
  plus bas), jamais donner accès à l'appli d'un autre utilisateur. Réalisation (`gateway::bridge::pipe_name`) :
  `mcp-proxy.<domaine>.<utilisateur>` lus dans `USERDOMAIN` et `USERNAME`, en minuscules, tout autre caractère
  remplacé par `_`, 64 caractères au plus par partie ; sans domaine, `mcp-proxy.<utilisateur>` ; sans nom
  d'utilisateur, aucun pipe : l'appli refuse de démarrer, le pont sort en erreur. Ces variables peuvent être
  changées par le client LLM : un nom faussé ne mène qu'à un autre pipe, où la preuve échoue.
- Création par l'API sûre de tokio : `tokio::net::windows::named_pipe::ServerOptions::new()
  .first_pipe_instance(true).reject_remote_clients(true).create(name)`, avec le **descripteur de sécurité par
  défaut**. Plus de DACL sur mesure ni de vérification du binaire client (`GetNamedPipeClientProcessId` n'existe
  qu'en `unsafe`, D18).
- Si le pipe existe déjà au démarrage (première instance refusée) : **échec fermé**, boîte de message claire
  (« un autre programme occupe le canal de MCP Proxy… ») et arrêt. L'instance unique (`tauri-plugin-single-instance`)
  a déjà traité le cas d'un second lancement de l'appli elle-même.
- **Preuve mutuelle par DPAPI**, avant tout octet MCP, à chaque connexion :
  1. chaque côté tire un nonce aléatoire de 32 octets, le protège par DPAPI utilisateur
     (`mcp_proxy_core::platform::windows::protect`) avec son rôle (`serveur` ou `pont`) et envoie ce **défi** ;
  2. chaque côté ouvre le défi reçu (`unprotect`), vérifie que le rôle est celui de l'autre côté (un défi renvoyé
     tel quel est refusé) et renvoie le nonce ;
  3. chaque côté compare en temps constant la réponse reçue à son propre nonce.

  Seul un processus du même utilisateur Windows peut ouvrir un défi. Cela protège le pont contre un pipe usurpé par
  un autre utilisateur, et l'appli contre un client d'un autre utilisateur. Messages encadrés par leur longueur,
  4 Kio au plus ; délai total de 5 s. Échec, message mal formé ou délai dépassé : connexion fermée, sans aucun
  octet MCP échangé, événement journalisé (sans nonce).
- La logique de la preuve vit dans `gateway::bridge`, générique sur un trait de preuve (`PeerProof` : protéger,
  ouvrir) : DPAPI sous Windows, faux en test. L'appli et le pont n'en font que le câblage.
  Réalisation : texte clair du défi `mcp-proxy.pipe.v1 ‖ NUL ‖ rôle ‖ NUL ‖ nonce` ; trames préfixées par leur
  longueur (`u32` gros-boutiste), de 1 à 4096 octets, une longueur annoncée hors bornes refusée avant toute lecture ;
  chaque côté envoie et lit en même temps (aucun blocage sur un tampon plein) ; DPAPI appelé hors de la tâche
  (`spawn_blocking`), le délai de 5 s couvrant aussi ces appels. Erreurs : `Timeout`, `Closed`, `Malformed`,
  `Foreign` (défi impossible à ouvrir), `Reflected`, `WrongAnswer`, `Protection`, `Random`, à textes fixes.
  Limite connue : rien ne lie la preuve à l'instance du pipe (voir `DECISIONS.md`, I011.5).
- Chaque connexion acceptée et prouvée devient un `GatewayServer::serve(pipe)` (`Gateway::serve_connection` : preuve
  en rôle serveur, puis `GatewayServer::run`). Chaque instance connectée est remplacée par une nouvelle avant
  d'être servie ; une instance impossible à créer est réessayée chaque seconde.
- Identité du client : `clientInfo` MCP. Le nom du processus parent du pont n'est pas lu : aucune API sûre ne le
  donne sans dépendance lourde (amendement du 2026-11-22).

### Mode `stdio`

- Se connecte au pipe. S'il n'existe pas : lance l'appli (même exe, sans argument, détachée) et attend le pipe
  jusqu'à 15 s.
- Preuve mutuelle (ci-dessus), puis relais par `gateway::bridge::relay`. Preuve échouée : message clair sur
  stderr (« le canal de MCP Proxy n'appartient pas à l'utilisateur courant »), code de sortie non nul. En cas d'échec : message clair sur stderr, code de sortie non nul.
- Réalisation : `gateway::bridge::prove_and_relay` ne lit rien de stdin avant la fin de la preuve. Pipe absent
  (`ERROR_FILE_NOT_FOUND`) : l'appli est lancée une fois (`DETACHED_PROCESS`, nouveau groupe de processus, sortie
  du job du client quand le job le permet, aucune poignée héritée), puis le pipe est réessayé toutes les 100 ms ;
  pipe occupé (`ERROR_PIPE_BUSY`) : réessai ; toute autre erreur : sortie en erreur. Messages en une ligne,
  préfixés « MCP Proxy : ».

### Assemblage au démarrage

Les boîtes de message passent par une API sûre (dialogue de Tauri ou crate équivalent, choisi à l'implémentation),
jamais par `MessageBoxW`. Réalisation : crate `rfd` 0.18 (`MessageDialog`), utilisable avant que Tauri ne démarre.

Ordre réel (amendement du 2026-11-22) : dossier des données (`%LOCALAPPDATA%\MCP Proxy\`), config, runtime
WebView2, puis construction de Tauri (instance unique d'abord : un second lancement s'arrête là), puis store de
secrets, `Gateway::start` (filtre, journal, hook de panic, réglages, registre, sessions, authentification, amonts,
suivi des listes, chaîne), exclusion WER et pipe, avant la boucle d'événements ; enfin, au démarrage de la boucle
(`setup` de Tauri), fenêtre et icône. Le journal
ne pouvant exister sans le filtre ni le filtre sans le store, un échec qui précède le journal (config, WebView2,
DPAPI, filtre) est écrit par `runtime::record_startup_failure` : une ligne à texte fixe dans le fichier du jour
(`logs\mcp-proxy-AAAA-MM-JJ.log`) ; les suivants passent par le journal. Dans tous les cas : boîte de message,
puis arrêt avec un code non nul.

1. Config : erreurs de validation affichées dans une boîte de message et journalisées, puis arrêt.
2. Runtime WebView2 absent, détecté par `tauri::webview_version()` (erreur ou version illisible) : boîte de message
   explicite, puis arrêt.
3. Journal fichier filtré, hook de panic, exclusion des rapports d'erreur Windows **par le registre** : valeur DWORD
   `1` nommée d'après l'exe sous `HKCU\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications`
   (`winreg`), idempotente. Un échec est journalisé et signalé par `doctor` (spec 024), sans empêcher le démarrage.
4. `SecretStore::for_platform` : s'il renvoie une erreur (DPAPI ne répond pas), boîte de message explicite, puis
   arrêt ; aucun repli (spec 002). Puis `OutputFilter`, `PolicyEngine`, `SettingsStore` (fichier JSON simple), `ToolRegistry` avec cache
   fichier, `SessionRegistry`, `OnDemandAuthenticator`, chaîne de middlewares, serveur de pipe.
   Réalisation (`gateway::runtime::Gateway::start`) : délai de grâce des variantes retirées =
   `upstream_timeout_secs` + 10 s ; `JsonFileStore` sur `settings.json` (réglages illisibles : les plus stricts,
   alerte de niveau erreur) ; `FileCatalogCache` sur `cache\catalog\` (depuis la spec 014 : réglages et cache
   scellés, reçus par `RuntimeDeps::persistence`) ; `OriginGuard::from_config` ; `HttpProbe`
   borné par `upstream_timeout_secs` ; `Upstreams::from_config` avec `modules::builtin()` ; `ListWatcher` gardé
   vivant et abonné aux événements de session ; journal donné comme trace à la chaîne ; `follow_tool_changes`.
   Un échec de n'importe quelle étape arrête le démarrage : pas de passerelle sans filtre, journal ou chaîne.
5. `AuthUi` et `ConfirmationBroker` provisoires qui **échouent fermé** (connexion impossible, confirmation refusée)
   jusqu'aux étapes 5b et 5c. Réalisation : `mcp_proxy_auth::UnavailableAuthUi` (toute fenêtre ou saisie rend
   `AuthUiError::Unavailable`) et `mcp_proxy_core::confirm::UnavailableConfirmation` (toute demande rend
   `Unavailable`). L'authentificateur traduit `AuthUiError::Unavailable` en `SessionError::Unavailable` : le LLM lit
   « connexion à « X » impossible pour le moment, appel refusé. », et l'état de la session revient à ce qu'il était.
   Une confirmation hors chat donne « confirmation impossible, aucun moyen de la demander à l'utilisateur ; appel
   refusé. ».

### Vue pour l'interface

- `AppSnapshot` : tout ce que l'interface affiche (sessions, environnements, MCP et outils, profil, alertes,
  mise à jour, version). Construit dans un crate (`gateway::view`), sérialisé en camelCase, **sans aucun secret**.
- Commande `get_snapshot` et événement `snapshot-changed`.
- Réalisation : `AppSnapshot::build(&SnapshotInputs)` lit la config, le registre, les réglages, l'état des
  sessions (jamais le store), l'état des instances et les mises à jour du `ListWatcher`, et les alertes. Contenu :
  `version`, `status`, `tooltip`, `profile` (`chosen`, `active`, `firstLaunch`, `choices`), `environments`
  (`exposed`), `sessions` (état : `kind`, `label`, `at` en millisecondes Unix ; services couverts), `mcps` et leurs
  instances (état, source de la liste, dernière erreur déjà filtrée, outils, mise à jour en attente),
  `exposedTools`, `exposedToolsThreshold`, `gatewaySuspended`, `pendingConfirmations` (0 avant la spec 013),
  `alerts`, `update` (`null` avant la spec 020). Sessions, environnements et instances dans l'ordre de la config.
  Avant de partir vers la fenêtre, le JSON repasse par le filtre de sortie (`Gateway::snapshot_json`) ; filtre
  indisponible : rien n'est envoyé. `snapshot-changed` porte l'instantané et n'est émis que vers la fenêtre
  `main`, à chaque changement du registre, d'une session, d'une instance, des réglages ou des alertes ; l'icône
  suit le même signal. Les libellés « active jusqu'à HH:MM » sont calculés en UTC à cette étape : l'interface
  doit afficher l'heure locale à partir de `at`.

## Tests (sous Linux, dans les crates)

- **EX-011-01** — `AppStatus` : neutre, attention, erreur selon les sessions, confirmations et erreurs en attente.
- **EX-011-02** — `AppSnapshot` : contenu attendu pour la config d'exemple ; aucun champ secret (test avec valeur sentinelle).
- **EX-011-03** — Analyse des arguments de lancement.
- **EX-011-09** — Preuve mutuelle de `gateway::bridge`, avec une fausse preuve et des flux en mémoire : deux côtés
  du même « utilisateur » passent puis relaient ; avec un côté d'un autre « utilisateur » (défi impossible à
  ouvrir), une réponse fausse, un défi renvoyé tel quel, un message trop long ou mal formé, ou un délai dépassé, la
  connexion est fermée **sans qu'aucun octet MCP soit relayé** dans un sens ou dans l'autre.

- **EX-011-13** — `AuthUi` et `ConfirmationBroker` provisoires, chaîne complète : un appel qui demande une
  connexion est refusé avec « connexion à « SSO Outils » impossible pour le moment, appel refusé. » et la session
  reste « jamais connectée » ; un appel qui demande une confirmation hors chat est refusé avec « confirmation
  impossible… ».
- **EX-011-14** — Nom du pipe : réduction, domaine absent, utilisateur absent, longueur bornée, aucun séparateur de
  chemin.
- **EX-011-15** — Échecs de démarrage : version de WebView2 lisible exigée ; chaque échec a un message explicite ;
  `record_startup_failure` écrit une ligne dans le fichier du jour.
- **EX-011-16** — Assemblage sous Linux (`Gateway::start` avec la config embarquée, dossier temporaire) : un client
  prouvé liste les outils exposés et en appelle ; un client d'un autre « utilisateur » est fermé avant tout octet
  MCP.

## Vérification

`cargo xtask ci` vert, contrôle Windows de `app/` compris.

## Recette manuelle (Windows)

- **EX-011-04** (manuel) — L'exe démarre sans fenêtre, l'icône apparaît ; « Ouvrir » affiche la fenêtre ; la fermer laisse l'icône.
- **EX-011-05** (manuel) — Un second lancement ne crée pas de seconde instance et affiche la fenêtre.
- **EX-011-06** (manuel) — Un client MCP configuré avec `mcp-proxy.exe stdio` liste les outils du catalogue.
- **EX-011-07** (manuel) — Si l'appli n'est pas lancée, `mcp-proxy.exe stdio` la démarre.
- ~~**EX-011-08** (manuel) — Un autre utilisateur de la machine ne peut pas ouvrir le pipe.~~ (retiré le
  2026-11-22 : D18, le descripteur de sécurité par défaut laisse un autre utilisateur ouvrir le pipe ; c'est la
  preuve mutuelle qui le rejette, voir `EX-011-09` et `EX-011-11`)

- **EX-011-10** (manuel) — Pipe déjà créé par un autre programme avant le lancement de l'appli : l'appli affiche le
  message d'échec et s'arrête, sans servir de connexion.
- **EX-011-11** (manuel) — Depuis une session d'un autre utilisateur Windows : un client qui ouvre le pipe de
  l'utilisateur est déconnecté avant tout octet MCP ; un pont lancé face à un pipe créé par un autre utilisateur
  refuse de relayer et sort en erreur.
- **EX-011-12** (manuel) — Après le premier démarrage, la valeur `mcp-proxy.exe` = 1 existe sous
  `HKCU\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications`.
- **EX-011-17** (manuel) — Icône : lisible sur barre claire et sombre ; au premier lancement (sans `settings.json`)
  la fenêtre s'ouvre seule ; avec un `settings.json` illisible, l'icône est rouge et l'infobulle dit « 1 erreur ».
- **EX-011-18** (manuel) — « Quitter » arrête l'appli : un pont connecté se termine, et le journal du jour contient
  le démarrage, la connexion du client et l'arrêt, sans aucun secret.

## Hors périmètre

Fenêtres de login (5b), notifications (5c), contenu de l'interface (6).

## Amendements

- **2026-11-22** — Aucun `unsafe` dans `app/` (D18, demande de l'utilisateur). Le pipe est créé par l'API sûre de
  tokio (première instance exigée, clients distants rejetés, descripteur de sécurité par défaut) ; la DACL sur
  mesure et la vérification du binaire client (`GetNamedPipeClientProcessId`, `QueryFullProcessImageNameW`)
  disparaissent, remplacées par une preuve mutuelle par DPAPI dans `gateway::bridge`. Pipe déjà présent au
  démarrage : échec fermé. Exclusion WER par le registre au lieu de `WerAddExcludedApplication`, détection du
  runtime WebView2 par `tauri::webview_version()`, boîtes de message par une API sûre, arrêt si
  `SecretStore::for_platform` échoue. Critère retiré : `EX-011-08` ; ajoutés : `EX-011-09` à `EX-011-12`.
- **2026-11-22** — Réalisation (I011.1 à I011.16). Ajouts : précisions de réalisation dans chaque section ;
  critères `EX-011-13` à `EX-011-16` (tests sous Linux) et `EX-011-17`, `EX-011-18` (recette). Écarts :
  l'assemblage vit dans `gateway::runtime` (testé sous Linux) et non dans `app/` ; l'ordre de démarrage suit les
  dépendances réelles (store et filtre avant le journal) et les échecs qui précèdent le journal sont écrits par une
  ligne fixe dans le fichier du jour ; `AuthUiError::Unavailable` devient `SessionError::Unavailable` dans
  l'authentificateur (spec 010) ; le nom du processus parent du pont n'est pas lu ; boîtes de message par `rfd`.
  Raison : points laissés ouverts par le texte, tranchés par l'option la plus sûre, et contraintes de D18.
- **2026-11-22** — Spec 013 : `mcp_proxy_core::confirm::UnavailableConfirmation` est supprimé ; `RuntimeDeps` reçoit
  un `ConfirmationPresenter` au lieu d'un `ConfirmationBroker`, et la passerelle assemble elle-même le
  `NotificationBroker`. Les tests et assemblages sans fenêtre passent `UnavailablePresenter`, dont toute confirmation
  hors chat rend le même refus « confirmation impossible, aucun moyen de la demander à l'utilisateur ; appel
  refusé. ».
- **2026-11-22** — Spec 014 : `Gateway::start` ne crée plus lui-même le stockage des réglages ni le scellement du
  cache, il les reçoit par `RuntimeDeps::persistence` (`SealedSettingsStore` et `SealedCatalog` dans l'appli). Un
  `settings.json` écrit à la main n'est plus accepté : la recette choisit le profil par le menu « Profil » de
  l'icône. Comportement et critères de la 011 inchangés.
