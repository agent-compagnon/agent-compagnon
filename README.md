# MCP Proxy

Passerelle Windows sécurisée entre des clients LLM (Claude Desktop, VS Code/Copilot…) et des serveurs MCP internes.
Les serveurs MCP délèguent l'authentification à l'application : le LLM ne voit jamais aucun secret.

Mémoire du projet : [`specs/`](specs/README.md) — constitution, architecture et une spec par fonctionnalité.
Décisions : [`DECISIONS.md`](DECISIONS.md). Consignes pour les agents : [`CLAUDE.md`](CLAUDE.md).

## Prérequis

- Rust 1.97.0, installé automatiquement par `rustup` d'après `rust-toolchain.toml` (composants `clippy`, `rustfmt`,
  cible `x86_64-pc-windows-msvc`).
- Node 22 et npm 10, pour `ui/` (squelette depuis la spec 011, interface à l'étape 6).

Le développement se fait sous Linux. Le code Windows y est contrôlé à la compilation (`cargo clippy --target
x86_64-pc-windows-msvc`) mais jamais exécuté ni lié. `app/` (l'appli Tauri) n'est compilé que pour cette cible et
lit `ui/dist` : construire `ui/` d'abord (`cargo xtask ci` le fait). Sous Windows : `cd ui && npm ci && npm run
build`, puis `cargo build -p mcp-proxy --release`. Lancé hors de `%LOCALAPPDATA%\Programs\MCP Proxy\`, l'exe
propose de s'installer (spec 019) ; `mcp-proxy.exe --no-install` le fait tourner sur place.

## Commandes

| Commande | Effet |
|---|---|
| `cargo xtask ci` | Toutes les vérifications : `fmt --check`, clippy hôte, tests, `ui/` (`npm ci`, lint, typecheck, tests, build), clippy cible Windows, traçabilité, contrôle du pipeline (`check-ci`) |
| `cargo xtask ci lint` | `fmt --check`, les deux passes clippy, lint et typecheck de `ui/` (construit avant la passe Windows) |
| `cargo xtask ci test` | Tests hôte, tests et build de `ui/`, traçabilité, contrôle du pipeline |
| `cargo xtask icons` | Redessine les icônes de `app/icons/` (appli, zone de notification) |
| `cargo xtask trace` | Alignement specs ↔ tests ↔ `docs/recette.md` (voir `specs/README.md`) ; `--write-index` régénère l'index des specs, `--report` affiche la matrice critère → fichiers |
| `cargo xtask keygen --out <fichier>` | Crée une paire de clés de signature des mises à jour (spec 020) : la clé privée dans `<fichier>`, jamais affichée ; la clé publique s'affiche, à mettre dans `crates/deploy/keys/current.pub` (ou `next.pub` pour une rotation) |
| `cargo xtask sign --exe <exe> [--version X.Y.Z] [--critical true\|false] [--key-env NOM]` | Écrit `<exe>.sig`, le manifeste signé ; clé privée lue dans `NOM` (`ED25519_PRIVATE_KEY` par défaut), jamais affichée |
| `cargo xtask verify --exe <exe> [--sig <fichier>] [--public-key <hex>]` | Vérifie un manifeste et son exe avec ces clés, sinon avec les clés embarquées |
| `cargo xtask check-release [--exe <exe>]` | Refuse la clé de développement et l'absence de clé courante ; avec `--exe`, vérifie l'exe construit. Le build avec `MCP_PROXY_RELEASE=1` fait le même contrôle de clés |
| `cargo xtask check-tag <tag>` | Vérifie qu'un tag de release est `v` suivi de la version du workspace, `X.Y.Z` ou `X.Y.Z-rc.N` (spec 021, lancé par le pipeline avant le build) |
| `cargo xtask check-ci` | Vérifie `.gitlab-ci.yml` sans GitLab : YAML valide, structure, jobs de release réservés aux tags protégés, aucun outil de construction ni gestionnaire de paquets dans les jobs qui reçoivent les secrets, aucune trace des commandes, commandes `cargo xtask` et `update-tool` connues, variables documentées dans `docs/release.md` (spec 021) |
| `cargo test -p mcp-proxy-core` | Tests du core, dont la validation de la config embarquée |
| `cd ui && npm run dev` | Interface dans un navigateur, avec le backend simulé (`?scenario=sessions`, `premier-lancement`, … : `ui/src/api/mock.ts` ; fenêtre d'installation : `install.html?scenario=mise-a-jour`, … : `ui/src/install/mock.ts`) |
| `cd ui && npm run e2e` | Tests Playwright de l'interface sur `vite preview` du backend simulé, hors `cargo xtask ci` ; `CAPTURES_DIR=…` y ajoute les captures |
| `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p mcp-proxy-gateway --test snapshot_contract` | Réécrit `ui/src/api/snapshot.example.json` après un changement voulu de l'instantané |

`cargo xtask ci` s'arrête à la première étape en échec et sort avec un code non nul.

Publier une version : [`docs/release.md`](docs/release.md) (pipeline GitLab de la spec 021, déclenché par un tag
protégé `vX.Y.Z`).

## Organisation

```
crates/core              mcp-proxy-core       config (étape 1), secrets en mémoire (2a) ; filtre, politiques… à venir
crates/auth              mcp-proxy-auth       méthodes d'authentification (étape 4)
crates/gateway           mcp-proxy-gateway    registre, amont, middlewares, serveur MCP, pont, vue, assemblage
crates/deploy            mcp-proxy-deploy     installation (7a), updater (7b, clés dans keys/) ; désinstallation à venir
crates/testkit           mcp-proxy-testkit    faux MCP, faux SSO, faux Artifactory (étape 3b)
crates/ci-check          mcp-proxy-ci-check   contrôle de .gitlab-ci.yml sans GitLab (`cargo xtask check-ci`, 7c)
crates/modules/jira-local                     module MCP local Jira (étape 8)
app/                     mcp-proxy            appli Tauri et pont stdio, Windows uniquement, câblage seulement (5a)
ui/                                           interface Vue (socle et onglet Connexions : spec 016)
config/default.toml                           config embarquée dans l'exe, validée au démarrage
xtask/                                        cargo xtask
.gitlab-ci.yml                                pipeline : contrôles, puis build, signature et publication sur tag (7c)
docs/release.md                               publier une version : clés, variables, runners, rotation, échecs
```

## Configuration

`config/default.toml` est compilée dans l'exe (`include_str!`) et validée strictement au chargement par
`mcp_proxy_core::config::Config::embedded()`. Toutes les erreurs sémantiques sont rapportées ensemble, chacune avec
son chemin TOML (`mcp[2].instances.dev.url : …`). Les données livrées sont des exemples : tous les hôtes sont en
`*.example.internal`.
