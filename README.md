# MCP Proxy

Passerelle Windows sécurisée entre des clients LLM (Claude Desktop, VS Code/Copilot…) et des serveurs MCP internes.
Les serveurs MCP délèguent l'authentification à l'application : le LLM ne voit jamais aucun secret.

Mémoire du projet : [`specs/`](specs/README.md) — constitution, architecture et une spec par fonctionnalité.
Décisions : [`DECISIONS.md`](DECISIONS.md). Consignes pour les agents : [`CLAUDE.md`](CLAUDE.md).

## Prérequis

- Rust 1.97.0, installé automatiquement par `rustup` d'après `rust-toolchain.toml` (composants `clippy`, `rustfmt`,
  cible `x86_64-pc-windows-msvc`).
- Node 22 et npm 10, seulement quand `ui/` existera (étape 6).

Le développement se fait sous Linux. Le code Windows y est contrôlé à la compilation (`cargo clippy --target
x86_64-pc-windows-msvc`) mais jamais exécuté ni lié.

## Commandes

| Commande | Effet |
|---|---|
| `cargo xtask ci` | Toutes les vérifications : `fmt --check`, clippy hôte, tests, clippy cible Windows, traçabilité, puis `ui/` s'il existe |
| `cargo xtask ci lint` | `fmt --check` et les deux passes clippy |
| `cargo xtask ci test` | Tests hôte, traçabilité et vérifications `ui/` |
| `cargo xtask trace` | Alignement specs ↔ tests ↔ `docs/recette.md` (voir `specs/README.md`) ; `--write-index` régénère l'index des specs, `--report` affiche la matrice critère → fichiers |
| `cargo test -p mcp-proxy-core` | Tests du core, dont la validation de la config embarquée |

`cargo xtask ci` s'arrête à la première étape en échec et sort avec un code non nul.

## Organisation

```
crates/core              mcp-proxy-core       config (étape 1), secrets en mémoire (2a) ; filtre, politiques… à venir
crates/auth              mcp-proxy-auth       méthodes d'authentification (étape 4)
crates/gateway           mcp-proxy-gateway    registre, amont, middlewares, serveur MCP, pont (étape 3)
crates/deploy            mcp-proxy-deploy     installation, désinstallation, updater (étape 7)
crates/testkit           mcp-proxy-testkit    faux MCP, faux SSO, faux Artifactory (étape 3b)
crates/modules/jira-local                     module MCP local Jira (étape 8)
config/default.toml                           config embarquée dans l'exe, validée au démarrage
xtask/                                        cargo xtask
```

## Configuration

`config/default.toml` est compilée dans l'exe (`include_str!`) et validée strictement au chargement par
`mcp_proxy_core::config::Config::embedded()`. Toutes les erreurs sémantiques sont rapportées ensemble, chacune avec
son chemin TOML (`mcp[2].instances.dev.url : …`). Les données livrées sont des exemples : tous les hôtes sont en
`*.example.internal`.
