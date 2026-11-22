---
id: "001"
titre: "Workspace Cargo, xtask ci, CI GitLab minimale, config TOML embarquée et validée"
etape: "1"
statut: "implémentée"
depend_de: []
suivi: "github#2"
---

# 001 — Workspace Cargo, xtask ci, CI GitLab minimale, config TOML embarquée et validée

## Objectif

Poser le socle : un workspace qui compile, l'outillage de vérification, et la config embarquée avec validation
stricte et données d'exemple.

## À faire

### Workspace

- `Cargo.toml` racine : `resolver = "3"`, édition 2024, `[workspace.package]` (version `0.1.0`),
  `[workspace.dependencies]`, `[workspace.lints]` :
  - rust : `unsafe_code = "deny"` ;
  - clippy en `deny` : `unwrap_used`, `expect_used`, `panic`, `dbg_macro`, `print_stdout`, `print_stderr`, `todo`,
    `unimplemented`.
  - `clippy.toml` : `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests`, `allow-dbg-in-tests`,
    `allow-print-in-tests` à `true`. `xtask` autorise l'affichage.
- `rust-toolchain.toml` : canal `1.97.0`, composants `clippy` et `rustfmt`, cible `x86_64-pc-windows-msvc`.
- `.cargo/config.toml` : `target-feature=+crt-static` pour la cible MSVC ; alias `xtask = "run --package xtask --"`.
- Membres, tous compilables et vides pour l'instant : `crates/core` (`mcp-proxy-core`), `crates/auth`
  (`mcp-proxy-auth`), `crates/gateway` (`mcp-proxy-gateway`), `crates/deploy` (`mcp-proxy-deploy`),
  `crates/testkit` (`mcp-proxy-testkit`, `publish = false`), `crates/modules/jira-local`
  (`mcp-proxy-module-jira-local`), `xtask`. `app/` n'arrive qu'à l'étape 5a ; `default-members` exclura `app`.
- `README.md` (prérequis, commandes), `.gitignore`, `.editorconfig`, `rustfmt.toml`.
- `DECISIONS.md` initialisé avec les tables D1–D16 et S1–S22 du cadrage (aujourd'hui dans `DECISIONS.md`), puis une section « Décisions
  prises en cours d'implémentation » alimentée à chaque étape.

### xtask

- `cargo xtask ci` enchaîne : `fmt --check`, clippy hôte (membres par défaut), tests hôte, clippy cible Windows
  (`--workspace`), la traçabilité (spec 030), puis les vérifications `ui/` si `ui/package.json` existe. Arrêt à la première erreur, code de
  sortie non nul, résumé final lisible.
- `cargo xtask ci lint` (fmt et les deux clippy) et `cargo xtask ci test` (tests, traçabilité et `ui/`) découpent la même suite
  pour les deux jobs GitLab.

### CI GitLab minimale

- `.gitlab-ci.yml` avec les étapes `lint` et `test` sur une image Rust Linux, qui lancent `cargo xtask ci lint` et
  `cargo xtask ci test`, avec
  cache cargo. Les jobs de build Windows et de release viennent à l'étape 7c.

### Config embarquée

- `config/default.toml`, chargé par `include_str!` dans `mcp_proxy_core::config`.
- Modèle typé (serde, `deny_unknown_fields`) : `schema_version`, `[gateway]`, `[trace]`, `[webview]`, `[update]`,
  `[[environments]]`, `[[sessions]]`, `[[mcp]]` (avec `instances` et `tools`), `[policy]`, `[profiles.*]`.
- Identifiants typés : `EnvId`, `SessionId`, `McpId`, `McpAlias`, `ToolName`, `ProfileId` (newtypes validés).
  `EnvId`, `SessionId`, `McpId`, `McpAlias` et `ProfileId` : `[a-z0-9-]`, 1 à 32 caractères, sans `-` en tête ni en
  fin ; `ToolName` : 1 à 128 caractères `[A-Za-z0-9._-]`. `env` est réservé comme identifiant de session.
- `ConfigSource` : trait (`name()`, `read()`) avec la seule implémentation `Embedded`. C'est le point d'extension d'une future config
  signée distante (`027-config-signee.md`), non implémentée.
- **Validation stricte** au chargement, en deux phases :
  - **structure** (syntaxe TOML, types, clés obligatoires, clés inconnues, méthode d'authentification,
    `webview.profile`) : arrêt à la *première* erreur, rapportée avec son chemin TOML et sa ligne ; le détail reste
    le message de serde, en anglais ;
  - **sémantique**, ensuite : elle accumule *toutes* les erreurs, chacune avec le chemin TOML fautif et un message
    explicite en français. `decision`, `channel`, `category`, `unknown_tools`, `scope` et `kind` sont validés à
    cette phase.
- Règles sémantiques :
  - `schema_version = 1` ;
  - identifiants uniques et bien formés, alias MCP uniques ;
  - références croisées valides (session, environnement, module local, profil) ; un MCP ne nomme qu'une session
    globale, ou `env` pour la session de l'environnement de l'instance ;
  - exactement une session `scope = "env"` par environnement, au moins une session globale ;
  - URL en `https` (`http` toléré pour `localhost` et `127.0.0.1`, pour les tests), avec un hôte et sans
    identifiants ; `update.base_url` compris ;
  - instances : au moins une par MCP, `url` en `remote`, `base_url` en `local` ; `module` obligatoire en `local`,
    interdit en `remote` ;
  - `[policy]` : règle racine `default` obligatoire ; décisions, canaux et catégories connus ; `channel` exigé dès
    que `decision = "confirm"` et refusé avec `allow` et `deny` ; une règle qui vise un environnement, un MCP, une
    instance ou un outil non déclaré est une erreur ;
  - une règle plus souple qu'une règle verrouillée qui la couvre est une erreur (elle serait sans effet). Une règle
    de catégorie verrouillée ne couvre pas un outil déclaré sans catégorie ; une règle `default` ou d'outil
    verrouillée le couvre ;
  - exactement un profil `strictest = true`, avec `unknown_tools = "deny"` ;
  - noms exposés calculables et sans collision, dans la limite `tool_name_max_len` (entre 16 et 128) ;
  - `1 ≤ call_budget_secs < 60`, `confirmation_timeout_secs ≤ call_budget_secs`, `auth_timeout_secs ≤
    call_budget_secs` ; délais, intervalles, seuils et capacités au moins égaux à 1 ;
  - paramètres complets pour chaque méthode d'authentification : durées de vie entre 1 et 24 h, `cookie_names` non
    vide et sans doublon, `capture_header` et `capture_scheme` jetons HTTP valides, motifs d'URL en préfixe canonique
    suivi d'un `*` final facultatif, `allowed_navigation` non vide et contenant l'hôte de `login_url` et de
    `success_url_pattern` ;
  - origines autorisées non vides pour chaque session ;
  - `update.repository` et `update.path` : segments `[A-Za-z0-9._-]`, sans segment vide, `.` ni `..`.
- Données d'exemple : 8 environnements, 2 SSO globaux (`sso-corp` en `jwt_header`, `sso-tools` en `basic`), 1 SSO
  par environnement (`cookie`), 5 MCP fictifs (`kibana-logs`, `gitlab-ops`, `k8s-admin`, `db-readonly`, `jira` en
  module local) répartis sur les environnements avec des URL en `*.example.internal`, outils déclarés avec leur
  catégorie quand elle est connue d'avance, 3 profils (`developpeur`, `support`, `lecture-seule`).
- La catégorie d'un outil est facultative dans `[mcp.tools]` : un outil déclaré sans catégorie reste connu, et sa
  catégorie suit S11 à l'exécution.

### Schéma de référence

Il peut être ajusté, à condition de le noter dans `DECISIONS.md`. Écarts retenus : `expected_issuer` facultatif
pour `jwt_header` ; `description`, `sensitive`, `list_requires_auth`, `strictest`, `default_environments`,
`profiles.*.policy` et `category` facultatifs ; tout le reste est obligatoire.

```toml
schema_version = 1

[gateway]
call_budget_secs = 50
confirmation_timeout_secs = 30
auth_timeout_secs = 45
upstream_timeout_secs = 120
slow_call_threshold_secs = 5
tool_name_max_len = 64
exposed_tools_warning_threshold = 100
refresh_interval_secs = 300
list_ttl_secs = 60
elicitation_clients = []            # valeurs de clientInfo.name jugées fiables pour le canal elicitation

[trace]
ring_capacity = 500
ring_max_bytes = 33554432
log_retention_days = 7

[webview]
profile = "in_memory"               # in_memory | ephemeral_disk
os_primary_account_sso = false

[update]
base_url = "https://artifactory.example.internal/artifactory"
repository = "generic-release-local"
path = "mcp-proxy"
check_interval_hours = 6

[[environments]]
id = "prod"
label = "PROD"
description = "Production"
sensitive = true

[[sessions]]
id = "sso-corp"
label = "SSO Entreprise"
scope = "global"                    # global | env
[sessions.auth]
method = "jwt_header"               # jwt_header | cookie | basic
login_url = "https://portal.example.internal/"
capture_url_pattern = "https://api.example.internal/*"
capture_header = "Authorization"
capture_scheme = "Bearer"
expected_audience = "api://internal"                # facultatif
allowed_navigation = ["portal.example.internal", "login.example.internal"]
max_ttl_hours = 24

[[sessions]]
id = "sso-prod"
label = "SSO PROD"
scope = "env"
environment = "prod"
[sessions.auth]
method = "cookie"
login_url = "https://sso.prod.example.internal/login"
cookie_url = "https://apps.prod.example.internal/"
cookie_names = ["SESSION"]
success_url_pattern = "https://apps.prod.example.internal/*"
allowed_navigation = ["sso.prod.example.internal", "apps.prod.example.internal"]
ttl_hours = 8

[[sessions]]
id = "sso-tools"
label = "SSO Outils"
scope = "global"
[sessions.auth]
method = "basic"
ttl_hours = 8
probe_url = "https://jira.example.internal/rest/api/2/myself"   # facultatif : valide les identifiants

[[mcp]]
id = "kibana-logs"
alias = "kibana"
label = "Kibana Logs"
kind = "remote"                     # remote | local
session = "env"                     # "env" = SSO de l'environnement de l'instance, sinon id d'une session globale
list_requires_auth = true
rate_limit = { per_minute = 60, burst = 10 }
[mcp.instances]
dev = { url = "https://kibana-mcp.dev.example.internal/mcp" }
prod = { url = "https://kibana-mcp.prod.example.internal/mcp" }
[mcp.tools]
search_logs = { category = "read" }
delete_index = { category = "destructive" }

[[mcp]]
id = "jira"
alias = "jira"
label = "Jira"
kind = "local"
module = "jira-local"
session = "sso-tools"
[mcp.instances]
prod = { base_url = "https://jira.example.internal" }
[mcp.tools]
search_issues = { category = "read" }
add_comment = { category = "write" }

[policy]                            # arbre de base : racine → env → mcp → tool
default = { decision = "confirm", channel = "notification" }
read = { decision = "allow" }
destructive = { decision = "deny" }
[policy.env.prod]
write = { decision = "confirm", channel = "notification", locked = true }
[policy.env.prod.mcp.kibana-logs.tool.delete_index]
rule = { decision = "deny", locked = true }

[profiles.developpeur]
label = "Développeur"
default_environments = ["dev", "int", "rec"]
unknown_tools = "confirm"           # confirm (notification obligatoire) | deny
[profiles.developpeur.policy.env.rec]
default = { decision = "allow" }

[profiles.lecture-seule]
label = "Lecture seule"
strictest = true                    # profil de repli quand les réglages sont invalides
default_environments = ["rec"]
unknown_tools = "deny"
[profiles.lecture-seule.policy]
write = { decision = "deny", locked = true }
destructive = { decision = "deny", locked = true }
```

## Tests

- **EX-001-01** — La config embarquée se charge et passe la validation.
- **EX-001-02** — Un test par règle de validation, sur un TOML minimal fautif ; le message cite le chemin fautif.
- **EX-001-03** — Plusieurs erreurs sont rapportées ensemble, pas seulement la première.
- **EX-001-04** — Une clé inconnue est refusée.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Secrets, filtre, politiques (étape 2). `app/` et `ui/` (étapes 5 et 6).

## Amendements

- **2026-11-22** — Rattrapage après implémentation (spec 030). Le texte décrit désormais ce qui a été livré :
  - les erreurs de **structure** s'arrêtent à la première (serde), seules les erreurs **sémantiques** sont
    accumulées : `EX-001-03` porte sur ces dernières (I1.1, I1.2) ;
  - la catégorie d'un outil est facultative, et une règle de catégorie verrouillée ne couvre pas un outil sans
    catégorie (I1.34) ;
  - règles de validation ajoutées ou précisées par l'option la plus sûre : URL, motifs, navigation, durées de vie,
    cibles de politique, `channel`, profil le plus strict, délais, longueur des noms, chemins de mise à jour,
    version de schéma (I1.14 à I1.27) ; `McpAlias` et identifiant de session `env` réservé (I1.5 à I1.8) ;
  - écarts de schéma (I1.10 à I1.12) et sous-commandes `ci lint` / `ci test` (I1.30).
- **2026-11-22** — `cargo xtask ci` lance aussi la vérification de traçabilité, après la passe clippy en cible
  Windows ; elle fait partie de `ci test` (spec 030).
