---
id: "011"
titre: "app — squelette Tauri, pont stdio (named pipe), instance unique, tray, réglages"
etape: "5a"
statut: "prévue"
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
- `unsafe` autorisé uniquement dans `app/src/platform/`, avec commentaires `// SAFETY:`.
- **Câblage seulement.** Toute logique testable vit dans `crates/` : si un morceau de cette issue peut être testé
  sous Linux, il va dans un crate.

### Modes de lancement

| Commande | Rôle |
|---|---|
| `mcp-proxy.exe` | Appli : instance unique, icône de notification, passerelle |
| `mcp-proxy.exe stdio` | Pont : relaie stdin/stdout vers l'appli. Aucun secret, aucune logique |

Les commandes internes (`--install`, `--uninstall`, `doctor`) arrivent aux étapes 7.

### Tauri

- `tauri.conf.json` : `productName = "MCP Proxy"`, identifiant `com.mcpproxy.app`, `frontendDist = ../ui/dist`,
  bundle désactivé, aucune fenêtre déclarée (elles sont créées à l'exécution), CSP stricte.
- Capacités : la fenêtre principale n'a que les commandes dont elle a besoin. Aucune capacité `remote`.
- `tauri-plugin-single-instance` : un second lancement met la fenêtre principale au premier plan.
- Fenêtre principale créée cachée. Elle s'ouvre au clic sur l'icône, par le menu « Ouvrir », au premier lancement
  (choix du profil) ou au second lancement. La fermer la cache ; l'appli continue.
- `ui/` : squelette Vite + Vue 3 + TypeScript avec une page vide, pour que `ui/dist` existe. `cargo xtask ci`
  construit `ui/` avant le contrôle Windows.

### Icône de notification

- Trois états : neutre, attention (orange : une session ou une confirmation attend), erreur.
- Le calcul de l'état (`AppStatus`) est une fonction pure, dans un crate, testée sous Linux.
- Menu : « Ouvrir », « Quitter ». Infobulle avec l'état. Trois jeux d'icônes lisibles sur barre claire et sombre.

### Named pipe

- Nom : `\\.\pipe\mcp-proxy.<SID de l'utilisateur>`.
- Sécurité : DACL limitée au SID courant, `PIPE_REJECT_REMOTE_CLIENTS`, `FILE_FLAG_FIRST_PIPE_INSTANCE`. À chaque
  connexion, vérifier que le processus client est bien ce même exe (`GetNamedPipeClientProcessId` puis
  `QueryFullProcessImageNameW`) ; sinon, fermer.
- Chaque connexion acceptée devient un `GatewayServer::serve(pipe)`.
- Identité du client : `clientInfo` MCP, complété si possible par le nom du processus parent du pont.

### Mode `stdio`

- Se connecte au pipe. S'il n'existe pas : lance l'appli (même exe, sans argument, détachée) et attend le pipe
  jusqu'à 15 s.
- Relais par `gateway::bridge::relay`. En cas d'échec : message clair sur stderr, code de sortie non nul.

### Assemblage au démarrage

1. Config : erreurs de validation affichées dans une boîte de message native et journalisées, puis arrêt.
2. Runtime WebView2 absent : boîte de message explicite, puis arrêt.
3. Journal fichier filtré, hook de panic, exclusion des rapports d'erreur Windows (`WerAddExcludedApplication`).
4. `SecretStore`, `OutputFilter`, `PolicyEngine`, `SettingsStore` (fichier JSON simple), `ToolRegistry` avec cache
   fichier, `SessionRegistry`, `OnDemandAuthenticator`, chaîne de middlewares, serveur de pipe.
5. `AuthUi` et `ConfirmationBroker` provisoires qui **échouent fermé** (connexion impossible, confirmation refusée)
   jusqu'aux étapes 5b et 5c.

### Vue pour l'interface

- `AppSnapshot` : tout ce que l'interface affiche (sessions, environnements, MCP et outils, profil, alertes,
  mise à jour, version). Construit dans un crate (`gateway::view`), sérialisé en camelCase, **sans aucun secret**.
- Commande `get_snapshot` et événement `snapshot-changed`.

## Tests (sous Linux, dans les crates)

- **EX-011-01** — `AppStatus` : neutre, attention, erreur selon les sessions, confirmations et erreurs en attente.
- **EX-011-02** — `AppSnapshot` : contenu attendu pour la config d'exemple ; aucun champ secret (test avec valeur sentinelle).
- **EX-011-03** — Analyse des arguments de lancement.

## Vérification

`cargo xtask ci` vert, contrôle Windows de `app/` compris.

## Recette manuelle (Windows)

- **EX-011-04** (manuel) — L'exe démarre sans fenêtre, l'icône apparaît ; « Ouvrir » affiche la fenêtre ; la fermer laisse l'icône.
- **EX-011-05** (manuel) — Un second lancement ne crée pas de seconde instance et affiche la fenêtre.
- **EX-011-06** (manuel) — Un client MCP configuré avec `mcp-proxy.exe stdio` liste les outils du catalogue.
- **EX-011-07** (manuel) — Si l'appli n'est pas lancée, `mcp-proxy.exe stdio` la démarre.
- **EX-011-08** (manuel) — Un autre utilisateur de la machine ne peut pas ouvrir le pipe.

## Hors périmètre

Fenêtres de login (5b), notifications (5c), contenu de l'interface (6).
