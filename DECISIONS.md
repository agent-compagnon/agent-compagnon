# Décisions

Journal des décisions du projet. Les tables D et S reprennent `docs/issues/00-cadrage.md` (décisions du cadrage) ;
la dernière section est alimentée à chaque étape d'implémentation.

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

## Décisions prises en cours d'implémentation

### Étape 1 — workspace, xtask, CI minimale, config embarquée

**Chargement et erreurs**

| # | Décision |
|---|---|
| I1.1 | Chargement en deux phases. La **structure** (syntaxe TOML, types, clés obligatoires, clés inconnues via `deny_unknown_fields`) est contrôlée par serde : arrêt à la première erreur, rapportée avec son chemin (`serde_path_to_error`) et sa ligne ; le détail technique reste le message serde, en anglais. La **sémantique** est contrôlée ensuite : toutes les erreurs sont accumulées, chacune avec son chemin TOML et un message en français. |
| I1.2 | Pour que leurs erreurs soient accumulées, `decision`, `channel`, `category`, `unknown_tools`, `scope` et `kind` sont lus comme chaînes puis validés. `sessions.auth.method` (qui choisit la forme de la table) et `webview.profile` restent des énumérations serde, donc des erreurs de structure. |
| I1.3 | Chemins d'erreur : tableaux indexés (`mcp[2].instances.dev.url`), clés hors `[A-Za-z0-9_-]` entre guillemets. Les erreurs globales portent le chemin de la collection (`sessions`, `profiles`). |
| I1.4 | `ConfigSource` expose `name()` (nom affiché dans le rapport d'erreurs) et `read() -> Cow<str>`. Seule implémentation : `Embedded`. `Config::from_toml` sert aux tests. |

**Identifiants et références**

| # | Décision |
|---|---|
| I1.5 | Identifiants (`EnvId`, `SessionId`, `McpId`, `ProfileId`) : `[a-z0-9-]`, 1 à 32 caractères, ni premier ni dernier caractère `-`. |
| I1.6 | Ajout du newtype `McpAlias`, de même grammaire : l'alias entre dans le nom exposé et ne doit pas contenir `_`, qui reste un séparateur non ambigu dans `{env}_{alias}_{outil}`. |
| I1.7 | `ToolName` suit la règle MCP (1 à 128 caractères `[A-Za-z0-9._-]`), mais c'est ici une erreur et non un avertissement : la config est sous notre contrôle. |
| I1.8 | `env` est réservé comme identifiant de session (valeur spéciale de `[[mcp]].session`). Un MCP ne peut nommer qu'une session **globale** ; une session d'environnement s'obtient avec `session = "env"`. |
| I1.9 | Modules locaux connus : constante `BUILTIN_MODULES = ["jira-local"]` dans le core, en attendant le registre de l'étape 8. `Config::load` reçoit la liste en paramètre. |

**Schéma** (écarts par rapport au schéma de référence)

| # | Décision |
|---|---|
| I1.10 | `jwt_header` accepte `expected_issuer` (facultatif), attendu par l'étape 4. |
| I1.11 | `environments.description` (défaut vide), `environments.sensitive`, `mcp.list_requires_auth`, `profiles.strictest` (défaut `false`), `profiles.default_environments` et `profiles.policy` sont facultatifs. Tout le reste du schéma de référence est obligatoire. |
| I1.12 | Instances : `url` pour `kind = "remote"`, `base_url` pour `kind = "local"`, l'autre clé est refusée. Au moins une instance par MCP. `module` obligatoire en `local`, interdit en `remote`. |
| I1.13 | Types de politique (`Decision`, `Channel`, `Category`, `Rule`, `RuleSet`, `PolicyTree`) définis dans `core::config` avec l'ordre de sévérité ; l'étape 2c pourra les déplacer dans `core::policy`. `unknown_tools = "confirm"` vaut `Confirm(Notification)`. |

**Règles de validation ajoutées ou précisées** (option la plus sûre)

| # | Décision |
|---|---|
| I1.14 | URL : `https` ; `http` toléré seulement pour `localhost` et `127.0.0.1` (pas `::1`) ; hôte obligatoire ; identifiants dans l'URL refusés. S'applique aussi à `update.base_url`. |
| I1.15 | Motifs d'URL (`capture_url_pattern`, `success_url_pattern`) : préfixe d'URL suivi d'un `*` final facultatif, `*` interdit ailleurs. Le préfixe doit être écrit sous forme canonique, ce qui garantit que l'hôte est terminé : `https://a.example*` (qui couvrirait `a.example.evil`) est refusé. |
| I1.16 | `allowed_navigation` : au moins un nom d'hôte, en minuscules, sans schéma ni joker, sans doublon. L'hôte de `login_url` et celui de `success_url_pattern` doivent y figurer, sinon la fenêtre de login ne pourrait pas aboutir. |
| I1.17 | Durées de vie (`max_ttl_hours`, `ttl_hours`) entre 1 et 24 h pour toutes les méthodes (jetons d'un jour au plus). `cookie_names` non vide, sans doublon ; `capture_header` et `capture_scheme` sont des jetons HTTP valides. |
| I1.18 | Origines autorisées (S7) : origines des URL d'instance des MCP qui utilisent la session, plus `probe_url`. Une session sans aucune origine (aucun MCP ne l'utilise) est une erreur : « calculable » est compris comme « non vide ». |
| I1.19 | `channel` est refusé avec `allow` et `deny` (il serait sans effet). |
| I1.20 | Une règle de politique visant un environnement inconnu, un MCP inconnu, un MCP sans instance dans cet environnement, ou un outil non déclaré dans `[mcp.tools]` de ce MCP est une erreur : une faute de frappe ne doit pas faire disparaître une protection sans bruit. |
| I1.21 | Couverture par une règle verrouillée L d'une règle R : le nœud de L est celui de R ou un ancêtre ; `default` couvre tout sélecteur, une catégorie couvre la même catégorie et les règles d'outil des outils déclarés de cette catégorie, une règle d'outil couvre la règle du même outil dans une autre couche. R est refusée si elle est strictement plus souple que L. Les verrous de la base s'appliquent à la base et à tous les profils ; le verrou d'un profil ne s'applique qu'à ce profil (une règle de base plus souple n'est sans effet que sous ce profil). |
| I1.22 | Le profil `strictest`, profil de repli, doit avoir `unknown_tools = "deny"`. |
| I1.23 | Délais : `1 ≤ call_budget_secs < 60`, `confirmation_timeout_secs ≤ call_budget_secs`, et en plus `auth_timeout_secs ≤ call_budget_secs` (l'attente de connexion se fait dans le budget de l'appel). Tous les délais, intervalles, seuils et capacités valent au moins 1. |
| I1.24 | `tool_name_max_len` entre 16 et 128 (préfixe utile + suffixe de hachage ; limite MCP). |
| I1.25 | Nom exposé : `mcp_proxy_core::naming::exposed_tool_name`, réutilisable par l'étape 3a. Au-delà de la limite, troncature puis `_` et 6 caractères hexadécimaux du SHA-256 du nom complet **avant** passage en minuscules et nettoyage, pour que deux noms qui ne diffèrent que par la casse ou la ponctuation obtiennent des suffixes différents. La validation vérifie l'absence de collision sur les outils déclarés de toutes les instances ; les outils découverts à l'exécution relèvent de l'étape 3a. |
| I1.26 | `update.repository` et `update.path` : segments `[A-Za-z0-9._-]` séparés par `/`, sans segment vide, `.` ni `..`. |
| I1.27 | `schema_version` doit valoir 1 ; toute autre valeur est refusée. |

**Données d'exemple**

| # | Décision |
|---|---|
| I1.28 | `kibana-logs` (session `env`) a une instance dans les 8 environnements, ce qui donne une origine à chaque session d'environnement. `k8s-admin` (session `env`) : 7 environnements ; `gitlab-ops` et `db-readonly` (session `sso-corp`) ; `jira` (module local, `sso-tools`) en REC et PROD sur la même instance Jira. |
| I1.29 | Politique de base : le schéma de référence, plus `write` verrouillé en confirmation (notification) en PPROD, `destructive` verrouillé en refus pour `k8s-admin` en PROD et `run_select` de `db-readonly` en confirmation en PROD. Profil `support` ajouté (REC, PPROD, PROD ; `write` refusé pour `k8s-admin` en PROD). `developpeur` autorise par défaut en DEV et REC. |

**Outillage**

| # | Décision |
|---|---|
| I1.30 | `cargo xtask ci lint` (fmt + deux clippy) et `cargo xtask ci test` (tests + `ui/`) pour les deux jobs GitLab `lint` et `test` ; `cargo xtask ci` sans argument enchaîne tout dans l'ordre du cadrage. |
| I1.31 | Tout le workspace est `publish = false` (projet interne) ; `testkit` et `xtask` le déclarent explicitement. `rust-version = "1.97.0"` au niveau du workspace. |
| I1.32 | CI GitLab : image `rust:1.97.0`, `rustup toolchain install` lit `rust-toolchain.toml` (composants et cible Windows), cache de `CARGO_HOME` (dans le projet) et de `target/`, clé sur `Cargo.lock`. |
| I1.33 | `rustfmt.toml` : `max_width = 120`. |
