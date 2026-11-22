# Décisions

Journal des décisions du projet, **en ajout seul** : une décision n'est jamais réécrite ni supprimée. Si elle
change, la nouvelle décision s'ajoute avec sa date et l'ancienne est marquée « remplacée par … ». Les tables D et S
viennent du cadrage du 2026-11-22 ; la dernière section est alimentée à chaque étape d'implémentation.

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
| D17 | Organisation des specs (décidé en conversation le 2026-11-22, hors QCM) | Le dépôt est la seule mémoire du projet : specs dans `specs/`, critères d'acceptation numérotés (`EX-NNN-nn`) cités par les tests et vérifiés par `cargo xtask trace`, amendements datés, commits terminés par `Spec: NNN`. Les tickets GitHub ne sont que des pointeurs. Raisons : ne pas perdre la mémoire en changeant de plateforme, garder specs et code alignés, pouvoir retracer un changement |

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
| I1.21 | Couverture par une règle verrouillée L d'une règle R : le nœud de L est celui de R ou un ancêtre ; `default` couvre tout sélecteur, une catégorie couvre la même catégorie et les règles d'outil des outils déclarés de cette catégorie, une règle d'outil couvre la règle du même outil dans une autre couche. R est refusée si elle est strictement plus souple que L : sa propre décision ne pourrait jamais s'appliquer, le plancher la relevant toujours ; le message demande une règle au moins aussi stricte que le verrou cité. Les verrous de la base s'appliquent à la base et à tous les profils ; le verrou d'un profil ne s'applique qu'à ce profil (une règle de base plus souple n'est sans effet que sous ce profil). |
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

**Catégorie facultative** (après relecture)

| # | Décision |
|---|---|
| I1.34 | `category` est facultative dans `[mcp.tools]` (`ToolConfig.category: Option<Category>`). Un outil déclaré sans catégorie reste un outil connu ; sa catégorie suit S11 à l'exécution : annotations du serveur, sinon destructif. Une valeur de catégorie invalide reste une erreur. Pour la couverture des verrous (I1.21), une règle de **catégorie** verrouillée ne couvre pas la règle d'un outil sans catégorie statique (la catégorie n'est connue qu'à l'exécution) ; une règle `default` verrouillée ou une règle d'outil verrouillée la couvrent toujours. Exemple : `get_merge_request` et `add_merge_request_note` de `gitlab-ops` sont déclarés sans catégorie. |

### Étape 2a — store de secrets en mémoire

**Organisation et protection mémoire**

| # | Décision |
|---|---|
| I2a.1 | Tout vit dans `mcp_proxy_core::secrets` ; le seul code `unsafe` est `core::platform::windows` (`WindowsMemoryProtector`). `InsecureDevProtector` est dans `secrets`, compilé sous `cfg(not(windows))`, et émet un `tracing::warn!` à sa construction. `platform_protector()` et `SecretStore::for_platform()` choisissent le protecteur de la plateforme. |
| I2a.2 | `MemoryProtector` exige `Send + Sync`. Taille de bloc : constante `PROTECT_BLOCK_SIZE = 16`, dont l'égalité avec `CRYPTPROTECTMEMORY_BLOCK_SIZE` est vérifiée à la compilation. Tous les protecteurs, y compris celui de développement, refusent un tampon vide ou non aligné, pour qu'une erreur d'alignement se voie dès les tests Linux. Un secret vide occupe un bloc. |
| I2a.3 | Le tampon scellé n'est jamais déchiffré en place : `with_exposed` et `with_variants` déchiffrent une copie `Zeroizing`, et le verrou du store est relâché avant d'appeler la closure (pas d'interblocage si elle rappelle le store, pas de fenêtre où le tampon scellé serait en clair). |
| I2a.4 | Verrou empoisonné : récupéré plutôt que propagé, pour que `wipe` et `wipe_all` fonctionnent toujours. |
| I2a.5 | Accès de test « tampon à zéro après `Drop` » : sous `cfg(test)`, `Drop` recopie l'état final du tampon (après effacement) dans une variable locale au thread. Aucune lecture de mémoire libérée. |

**Store**

| # | Décision |
|---|---|
| I2a.6 | **Écart** : `with_variants` renvoie `Result<R, SecretError>` et non `R`. Si une variante ne peut être déchiffrée, la closure n'est pas appelée : filtrer avec une liste incomplète laisserait passer un secret, le filtre (2b) doit donc échouer fermé. |
| I2a.7 | `put` renvoie `Result` et est atomique : validation, scellement de la valeur et de toutes ses variantes, puis remplacement. En cas d'erreur, l'ancien secret de la session reste inchangé. `wipe` renvoie `true` s'il y avait un secret. |
| I2a.8 | Validation au `put`, pour qu'une injection d'en-tête soit impossible : valeurs secrètes non vides, 64 Kio au plus (`MAX_SECRET_LEN`), octets admis dans une valeur d'en-tête HTTP (aucun caractère de contrôle sauf la tabulation, donc ni CR, ni LF, ni NUL). Cookies : au moins un, nom conforme à la grammaire `token` HTTP, sans doublon, valeur sans `;`. Login Basic : non vide, sans `:` ni caractère de contrôle. Les motifs d'erreur sont des textes fixes, jamais la valeur. |
| I2a.9 | Point d'extension de 2b : `secrets::variants::derive_variants(&Credential)`. En 2a il ne renvoie que les valeurs brutes (jeton, mot de passe Basic, chaque valeur de cookie ; pas le login), non vides et dédupliquées ; elles sont scellées comme la valeur elle-même. |
| I2a.10 | `Debug` : `SealedSecret(***)` ; `Credential` et `ExposedCredential` affichent le type, les noms de cookies et le login Basic (non secrets), jamais une valeur ; `SecretStore` n'affiche que les identifiants de session. L'appartenance d'un secret à une seule session tient à la clé de session et au fait que `Credential` n'est ni clonable ni copiable et est consommé par `put` ; deux sessions qui recevraient la même valeur ne sont pas détectées. |

**Origines et client HTTP**

| # | Décision |
|---|---|
| I2a.11 | `OriginGuard::from_config` reprend `Session.allowed_origins` calculé à l'étape 1 (I1.18 : URL d'instance des MCP distants, `base_url` des modules locaux, `probe_url`). `authorize` refuse une session inconnue, une URL avec identifiants (`user:pass@`), un schéma autre que `https`, et toute origine non déclarée. `http` n'est accepté que pour `localhost` et `127.0.0.1`, comme pour les URL de la config (I1.14), et seulement si l'origine est déclarée. L'erreur cite l'origine refusée (schéma, hôte, port), jamais le chemin ni la requête. |
| I2a.12 | API du client : `request(Method, Url) -> reqwest::Request`, puis `execute(Request) -> reqwest::Response`. `authorize` puis lecture du secret ont lieu avant tout envoi ; les en-têtes `Authorization` et `Cookie` posés par l'appelant sont retirés, l'en-tête injecté est marqué `sensitive`. Retirés de la réponse : `Set-Cookie`, `Authorization`, `WWW-Authenticate`, `Proxy-Authenticate`, plus `Proxy-Authorization` et `Set-Cookie2`. |
| I2a.13 | Le délai du client (constructeur) borne chaque requête, connexion comprise ; une requête peut demander un délai plus court, jamais plus long. Aucune redirection, `Referer` désactivé, backend TLS natif choisi explicitement. Les erreurs `reqwest`, dont le texte contient l'URL, sont réduites à `Timeout`, `Connect` ou `Transport`. |
| I2a.14 | Proxy : le client des origines distantes garde les proxys de l'environnement (nécessaires en entreprise ; en `https` le proxy ne voit qu'un tunnel `CONNECT`). Les requêtes vers `localhost`/`127.0.0.1` passent par un second client sans aucun proxy, pour qu'un en-tête d'authentification en `http` clair ne soit jamais remis à un proxy. |
| I2a.15 | Limite connue : la valeur injectée est copiée dans un `HeaderValue` que `reqwest`/`hyper` ne remettent pas à zéro. Les tampons intermédiaires construits par le core (`Bearer …`, `login:motdepasse`, base64, liste de cookies) sont `Zeroizing`. |
| I2a.16 | `AuthenticatedHttpClient::new(session, store, guard, timeout)` est public et refuse une session inconnue du garde ; le câblage vers les modules locaux viendra avec les étapes 3 et 8. Le trait `SessionGate` (3b) n'est pas créé. |

### Spec 030 — traçabilité des specs

| # | Décision |
|---|---|
| I030.1 | Dans le code, seule la forme `// Exigence : EX-NNN-nn` compte comme citation (mot `Exigence`, deux-points, un ou plusieurs identifiants séparés par des virgules). Un identifiant écrit ailleurs (donnée de test, texte) n'est pas une citation : sans cela, les jeux d'essai de `xtask` seraient des citations d'identifiants inconnus. Les tests de `xtask` construisent leurs citations à l'exécution. Dans `docs/recette.md`, tout identifiant compte. Une citation mal formée est une erreur. |
| I030.2 | En-têtes de spec : analyseur strict écrit à la main, sans dépendance YAML (`xtask` reste sans dépendance). Clés admises : `id`, `titre`, `etape`, `statut`, `depend_de`, `suivi` ; valeur `"texte"` sans guillemet intérieur ni échappement, ou liste `["a", "b"]`. Clé inconnue, clé en double, valeur mal écrite, `id` qui n'est pas trois chiffres : erreur. |
| I030.3 | Critère : puce `-` ou `*`, indentée ou non (les sous-puces de la spec 004 en sont), qui commence par `**EX-NNN-nn**` (exactement trois puis deux chiffres) ; `(manuel)` juste après l'identifiant ; critère retiré : puce qui commence par `~~`. Les blocs de code délimités sont ignorés. Tout fichier de `specs/` dont le nom commence par un chiffre est une spec et doit s'appeler `NNN-nom.md`. |
| I030.4 | Spec à l'en-tête invalide : erreur, mais ses critères restent connus (pas d'erreurs en cascade sur leurs citations) ; elle sort de l'index, et `--write-index` refuse d'écrire tant qu'un nom ou un en-tête est invalide, pour qu'une spec cassée ne disparaisse pas de l'index sans bruit. |
| I030.5 | Index : colonne « Critères » = nombre de critères non retirés ; « Étape » vaut `—` sans `etape` ; dans « Dépend de », une suite d'au moins quatre numéros consécutifs s'écrit `001 à 010`. Cette règle reproduit l'index existant à l'identique (`001 à 010` pour 011, mais `007, 010, 011, 012` pour 015). |
| I030.6 | Fichiers de code : `git ls-files` (suivis seulement : un nouveau test doit être ajouté à l'index git pour compter), sous `crates/`, `app/`, `xtask/`, `ui/`, hors dossiers `target`, `node_modules` et `dist`. Un fichier suivi mais supprimé de l'arbre de travail est ignoré. Si `git` échoue, la vérification échoue. |
| I030.7 | Un critère automatique cité seulement dans la recette n'est pas couvert ; un critère manuel cité dans le code ne compte pas, sans être une erreur. |
| I030.8 | Dans `cargo xtask ci`, « traçabilité » se place après la passe clippy en cible Windows et avant `ui/`, dans l'ordre donné par `CLAUDE.md`, et appartient au groupe `ci test` (job GitLab `test`). L'étape relance le binaire `xtask` courant plutôt que de repasser par cargo. |
| I030.9 | Le test `EX-030-10` lit le dépôt réel (via `git`) : il échoue dès que le dépôt n'est plus aligné, ce qui fait aussi échouer `cargo test`. |
| I030.10 | Rattrapage 001 : `EX-001-02` (un test par règle) est cité une fois en tête de chaque section de tests de validation, plus sur `unsupported_schema_version`, plutôt que sur chacun des tests. Le texte de `EX-001-03` n'est pas reformulé (identifiants stables) alors que seules les erreurs sémantiques sont accumulées : l'amendement du 2026-11-22 le précise. |
| I030.11 | Rattrapage 002 : les trois vérifications Windows deviennent `EX-002-07` à `EX-002-09` ; `docs/recette.md` a une section par spec et une case `- [ ] **EX-…** — …` par critère manuel. |

### Spec 002 — variantes retirées (amendement du 2026-11-22)

| # | Décision |
|---|---|
| I002.1 | Après `wipe` ou `wipe_all`, le secret n'est plus injectable, mais ses variantes restent scellées et exposées par `with_variants` pendant un délai de grâce, puis sont effacées. Raison : une réponse à un appel parti avant l'effacement peut encore contenir le secret et doit rester masquée. |
| I002.2 | Option la plus sûre : le **remplacement** d'un secret par `put` retire aussi les variantes de l'ancien (cas du renouvellement d'un jeton pendant un appel). Un `put` ne raccourcit jamais le délai des variantes déjà retirées. |
| I002.3 | Délai : `SecretStore::with_variant_grace`, défaut `DEFAULT_VARIANT_GRACE` = 130 s (`upstream_timeout_secs` de la config embarquée + 10 s) en attendant le câblage par la passerelle (étape 3). Horloge injectable : trait `Clock` (`SystemClock` par défaut, `with_clock`). Pas de minuterie : les variantes expirées sont effacées à la première opération qui les rencontre (`with_variants` purge sous verrou exclusif avant de déchiffrer). En attendant, elles restent scellées. Une variante retirée illisible fait échouer `with_variants`, comme une variante active. |

### Spec 003 — filtre de sortie

| # | Décision |
|---|---|
| I003.1 | Tout vit dans `mcp_proxy_core::filter` (`OutputFilter`, `FilterReport`, `HitKind`, `FilterError`, `FilteringMakeWriter`, marqueurs) ; le calcul des variantes reste dans `secrets::variants`, appelé par `put`. Dépendances ajoutées : `memchr` (recherche des variantes), `regex` (motifs génériques seulement : aucune valeur secrète n'est jamais compilée dans un automate), `serde_json`, `tracing-subscriber` (`fmt`, `std`) pour le trait `MakeWriter`. |
| I003.2 | **Échec fermé** (décidé à la relecture de 2a). `filter_json`, `filter_text` et `contains_known_secret` renvoient `Result<_, FilterError>` ; sur `Err`, l'appelant ne transmet rien, et un argument entrant est bloqué. `contains_known_secret` renvoie une erreur plutôt que `true`, pour que l'appelant puisse dire « filtre indisponible » sans mentir sur la présence d'un secret. En plus, `filter_json` remplace la valeur par `[SECRET MASQUÉ]` et le `MakeWriter` écrit une ligne fixe, pour qu'un appelant qui ignorerait l'erreur n'envoie rien. `FilterError` ne porte aucune valeur (seulement le `SecretError` d'origine, lui-même sans valeur). |
| I003.3 | `OutputFilter::new(Arc<SecretStore>, &FilterConfig) -> Result` : la compilation des motifs ne peut pas échouer en pratique, mais aucun `unwrap` hors des tests. |
| I003.4 | Variantes ajoutées au texte initial (option la plus sûre) : encodage base64/base64url complet, avec et sans `=`, en plus des trois fragments d'alignement (masque le bloc entier, pas seulement sa partie centrale) ; formes JSON `\/` et `\uxxxx` (Python) en plus de celle de `serde_json` ; signature, base64 et hexadécimal de la signature pour **toute** valeur en forme de JWT (cookie compris), base64 avec et sans `=` ; `login:motdepasse` brut en plus de son base64 ; forme pourcent-décodée d'un cookie traitée comme une valeur brute (non soumise à la longueur minimale, avec ses propres variantes). |
| I003.5 | Fragments d'alignement : seuls les caractères entièrement déterminés par le secret sont cherchés, donc le premier et le dernier caractère base64 qui encadrent le marqueur peuvent porter jusqu'à 6 bits du premier ou du dernier octet du secret. Accepté : élargir le masque à tout le bloc base64 masquerait des données légitimes (images) sans gain réel. |
| I003.6 | Masquage exact : occurrences qui se chevauchent ou se touchent fusionnées en un seul `[SECRET MASQUÉ]` ; une occurrence entièrement dans un marqueur existant est ignorée (sinon un secret court comme `MASQU` rendrait le filtre non idempotent) ; une occurrence qui chevauche un marqueur l'absorbe ; une occurrence qui coupe un caractère UTF-8 est élargie au caractère entier. Aiguilles empruntées à `with_variants`, triées et dédupliquées, jamais copiées. |
| I003.7 | Idempotence par construction : liste noire puis motifs génériques réappliqués jusqu'au point fixe. Chaque passe qui change le texte remplace au moins un octet hors marqueur ; au-delà de 8 passes (entrée pathologique), le texte entier devient `[SECRET MASQUÉ]`. |
| I003.8 | Un nombre JSON dont l'écriture décimale contient un secret connu devient une chaîne masquée (un code PIN numérique ne doit pas sortir). La règle « nombres et booléens jamais touchés » ne vaut que pour les champs sensibles. `contains_known_secret` regarde aussi les clés et les nombres. |
| I003.9 | Clés JSON : filtrées comme les chaînes. Deux clés qui ne différaient que par un secret et coïncident après masquage voient leurs valeurs regroupées dans un tableau, plutôt qu'un renommage (`clé (2)`) dont le suffixe pourrait correspondre à un secret court. Une clé est sensible si sa forme d'origine **ou** sa forme masquée l'est, pour que la seconde passe donne la même réponse. Parcours itératif (pile explicite) : un document très imbriqué ne fait pas déborder la pile. |
| I003.10 | Clés sensibles : découpage sur tout caractère non alphanumérique, aux frontières camelCase (`apiKey`, `APIKey`) et entre lettres et chiffres ; un dernier segment purement numérique est ignoré (`password2`, `token_1` sont sensibles). Les noms complets se comparent sous forme compacte (minuscules, sans séparateur) : `apiKey`, `API_KEY`, `x-api-key`, `accessKey` correspondent. `tokens`, `tokenizer`, `token_type`, `secretary` ne le sont pas. Sous un champ sensible, seule une valeur chaîne directe est masquée ; un objet ou un tableau est parcouru normalement. |
| I003.11 | `[filter]` (facultatif) : `extra_key_segments` et `extra_keys` **s'ajoutent** aux valeurs par défaut, qui ne peuvent pas être retirées (INV-6 : la config peut durcir le filtre, jamais l'assouplir). Validation : segments en lettres minuscules et chiffres ASCII, noms en lettres, chiffres, `_`, `-`, `.`, au moins une lettre, sans doublon (casse ignorée) ; clé inconnue refusée par `deny_unknown_fields`. |
| I003.12 | Texte : clé entre guillemets (`"…"`, `'…'`, `\"…\"`) = règle JSON (valeur chaîne seulement) ; clé nue = toute valeur masquée, car on ne peut pas distinguer un mot de passe numérique. `clé=valeur` s'arrête au premier blanc, `&`, `;`, `,`, guillemet, `<`, `>` ; `clé: valeur` va jusqu'à la fin de la ligne (sur-masquage accepté plutôt qu'un mot de passe à espaces coupé). `Authorization` et `Proxy-Authorization` : schéma conservé, valeur `[EN-TÊTE MASQUÉ]` jusqu'à la fin de la ligne (`Digest` a des virgules). Lignes `Cookie:` / `Set-Cookie:` / `Set-Cookie2:` reconnues où qu'elles soient dans la ligne (un log les cite rarement en début de ligne). Dans un objet JSON, `Authorization` est un champ sensible (`[CHAMP MASQUÉ]`). |
| I003.13 | Bonus réalisés : blocs `-----BEGIN … PRIVATE KEY-----` (aussi `PGP PRIVATE KEY BLOCK`), jusqu'à la fin du texte s'il manque la ligne `END`, et mot de passe d'URL (jusqu'au dernier `@` de l'autorité) ; tous deux masqués par `[SECRET MASQUÉ]`, sans nouveau marqueur. `FilterReport.kinds` est trié et sans doublon ; le rapport ne donne ni la valeur ni sa position. |
| I003.14 | `FilteringMakeWriter` : tampon par enregistrement (`Zeroizing`, agrandi par recopie dans un nouveau tampon pour qu'aucune réallocation ne laisse de copie), filtré et écrit quand l'écrivain est relâché ou vidé ; octets non UTF-8 remplacés (`from_utf8_lossy`). Limite connue : le tampon de formatage interne de `tracing_subscriber::fmt` contient le texte avant filtrage et n'est pas effacé. |
| I003.15 | Limites connues, non traitées : un secret qui est une sous-chaîne d'un marqueur (`SECRET`, `MASQU`…) ne se distingue pas du marqueur ; un secret brut d'un caractère masque toutes ses occurrences ; un secret coupé entre deux champs JSON, réencodé autrement (base32, gzip, double base64, casse changée) ou dans du XML (`<password>…</password>`) n'est pas reconnu par la liste noire ; `Bearer <jeton>` sans nom d'en-tête n'est reconnu que si le jeton est un JWT ou un secret connu. |

### Spec 004 — moteur de politiques, profils et réglages

| # | Décision |
|---|---|
| I004.1 | Module `mcp_proxy_core::policy` : `PolicyEngine`, `Resolution`, `Provenance`, `DecidedBy`, `RuleOrigin`, `Layer`, `UnknownTools`, `ToolAnnotations`, `CategorySource`, `PolicySummary`, `CategoryPolicy`. Les types de règles restent dans `config::policy` (validés au chargement) et sont réexportés par `policy`. Aucune dépendance ajoutée. |
| I004.2 | Position d'une règle définie une fois (`config::RulePosition`, `Selector`, `Level`) : `applies_to` pour le moteur, `covers` pour la validation (I1.21), `PolicyTree::rules()` pour aplatir un arbre. `validate::FlatRule` s'appuie désormais dessus ; les messages et chemins d'erreur sont inchangés. Le test `floor_agrees_with_validation_on_the_embedded_config` (EX-004-09) vérifie que les deux racontent la même histoire. |
| I004.3 | Le moteur emprunte la config, le profil et l'arbre de durcissement (`PolicyEngine<'a>`) : il ne garde aucun état et se reconstruit à chaque changement de réglages. |
| I004.4 | Échec fermé : sans aucune candidate (arbre sans racine `default`, impossible après validation), la décision est `Deny` et `decided_by` vaut `NoRule`. |
| I004.5 | Déterminisme : la précision `(sélecteur, profondeur, couche)` est unique parmi les candidates d'un appel ; le plancher est la candidate verrouillée maximale selon `(sévérité, précision)`. La provenance garde le plancher même quand il ne relève rien ; `decided_by` ne retient une étape postérieure (plancher, `unknown_tools`, durcissement) que si elle est strictement plus sévère. |
| I004.6 | Outil connu = déclaré dans `[mcp.tools]` d'un MCP qui a une instance dans l'environnement de l'appel. Sinon (MCP inconnu, sans instance là, outil absent) : inconnu, donc `Destructive` et `unknown_tools` (option la plus stricte). |
| I004.7 | Annotations : `ToolAnnotations { read_only_hint, destructive_hint }` (champs `Option<bool>`). Sans aucun des deux indices, elles comptent comme absentes : catégorie `Destructive`, source `Default` (et non `Annotation`, qui serait trompeur). |
| I004.8 | Arbre utilisateur : `locked` y est ignoré (pas de plancher utilisateur). Le point 6 appliquant l'ordre de précision à l'arbre utilisateur seul, une règle utilisateur plus précise et plus souple remplace une règle utilisateur plus large : c'est voulu par l'algorithme validé (D5, D6) et ne contredit pas INV-6, la décision ne descendant jamais sous celle de la config. EX-004-03 est prouvé dans ce sens (avec durcissement ≥ sans durcissement, quel que soit l'arbre utilisateur). À relire : l'interface de durcissement (spec 017) devra montrer cet effet. |
| I004.9 | Rendu français : `Decision::label` (`autorisé`, `confirmation (elicitation)`, `confirmation (notification)`, `refusé`) ; libellés d'environnement, de MCP et de profil pris dans la config (identifiant à défaut) ; `describe` court, `describe_detailed` avec les règles écartées et la catégorie. |
| I004.10 | Module `mcp_proxy_core::settings` : `UserSettings`, `SettingsStore`, `JsonFileStore`, `LoadedSettings`, `SettingsOrigin`, `SettingsError`, `SETTINGS_SCHEMA_VERSION`. Options en champs directs (`autostart`, `beta_channel`, `gateway_suspended`), comme les citent les specs 015, 019 et 020. MCP coupés par MCP, dans tous les environnements (`disabled_mcp`). Valeurs par défaut (= les plus strictes) : aucun profil, rien d'exposé, rien de coupé, aucun durcissement, toutes les options à `false`. `UserSettings::engine(&Config)` renvoie `None` seulement pour une config sans profil `strictest`, que la validation refuse ; l'appelant échoue fermé. |
| I004.11 | `SettingsStore::load` ne renvoie jamais d'erreur : `LoadedSettings { settings, origin }` avec `Stored`, `Missing` ou `Reset(SettingsError)`. Est un `Reset` : erreur d'entrée-sortie, JSON non conforme, champ manquant ou inconnu (tous les champs sont obligatoires), identifiant invalide, règle incohérente, `locked` dans une règle utilisateur, version de schéma autre que 1, fichier de plus de 1 Mio. Les durcissements d'un fichier illisible sont perdus : sans profil choisi rien n'est exposé, donc rien n'est plus souple. Les messages ne citent aucune valeur du fichier (position JSON seulement ; le message de serde, qui peut citer une valeur, n'est pas repris). Les règles du durcissement ne sont pas validées contre la config : une règle visant un environnement, un MCP ou un outil inconnus ne peut que durcir, elle est gardée. |
| I004.12 | Un profil choisi qui n'existe plus dans la config est traité comme absent : profil `strictest` et aucun environnement exposé, jusqu'à un nouveau choix. Un environnement exposé inconnu de la config est ignoré. `choose_profile` remet les environnements exposés aux `default_environments` du profil et garde les durcissements (spec 016). |
| I004.13 | `JsonFileStore::save` crée le dossier parent, écrit `<fichier>.tmp`, le synchronise puis le renomme ; en cas d'échec, le fichier temporaire est supprimé au mieux. |

### Spec 002 — enveloppe (amendement du 2026-11-22, D18)

| # | Décision |
|---|---|
| I002.4 | AEAD : XChaCha20-Poly1305 (`chacha20poly1305` 0.11, features `alloc` et `zeroize`, sans `getrandom` du crate) plutôt que ChaCha20-Poly1305 : nonce de 192 bits tiré au hasard à chaque scellement sans risque de collision. Aléa : `getrandom` 0.4 (`getrandom::fill`), pour la clé de session comme pour les nonces ; un échec du générateur est une erreur (`ProtectionFailure::Random`). Le chiffrement se fait en place dans un tampon `Zeroizing` dont la capacité réserve déjà l'étiquette, pour qu'aucune réallocation ne laisse de copie en clair ; le déchiffrement aussi, sur une copie. |
| I002.5 | Données associées : `mcp-proxy.secret.v1`, NUL, identifiant de session, NUL, place de la valeur (`bearer`, `cookie:<nom>`, `basic:<login>`, `variant`). Option la plus sûre : la place lie aussi le nom de cookie et le login Basic, qui sont gardés en clair à côté. Les variantes retirées gardent l'identifiant de leur session pour pouvoir être ouvertes. Toute erreur d'ouverture AEAD est `SecretError::Corrupted` (texte inchangé). |
| I002.6 | `KeyWrapper { wrap(&[u8]) -> Result<Vec<u8>>, unwrap(&[u8]) -> Result<Zeroizing<Vec<u8>>> }`. Implémentations : `DpapiKeyWrapper` (Windows, s'appuie sur `core::platform::windows::{protect, unprotect}`, erreur `DpapiError(HRESULT)`) et `InsecureDevKeyWrapper` (hors Windows, garde la clé telle quelle, même avertissement qu'avant, mot pour mot, pour que `EX-002-09` reste vrai). `platform_key_wrapper()` choisit. Une clé déballée de longueur autre que 32 octets est refusée (`ProtectionFailure::InvalidKey`). |
| I002.7 | **Écart d'API** : `SecretStore::new(Box<dyn KeyWrapper>)` et `SecretStore::for_platform()` renvoient `Result<SecretStore, SecretError>`. L'aller-retour d'essai est fait par `new`, donc pour tout enrobage ; un échec y devient `SecretError::KeyProtectionUnavailable(ProtectionFailure)` (« protection des secrets indisponible … aucun secret ne sera gardé »), y compris `RoundTrip` si l'enrobage rend une autre clé. L'appelant (étape 5a) doit refuser de démarrer la gestion des secrets plutôt que de se replier. |
| I002.8 | Un déballage par usage, fait **hors du verrou** du store ; la clé en clair (`ClearKey`, effacée au `Drop`) est relâchée dès que les valeurs sont scellées ou déchiffrées, donc **avant** la closure (plus strict que « après »). Chaque opération AEAD construit son propre `XChaCha20Poly1305`, qui efface sa copie de la clé au `Drop` (feature `zeroize`). `with_exposed` vérifie d'abord la présence d'un secret : aucun appel DPAPI pour une session vide. Accès de test : `key_probe` (état de chaque `ClearKey` au `Drop`), comme `drop_probe`. |
| I002.9 | Limite connue : les tampons WinRT (`IBuffer`) qui portent la clé en clair vers et depuis DPAPI sont libérés par le runtime sans être effacés, et rien ne permet de les effacer sans `unsafe` ; seules les copies faites par le code (tableau intermédiaire, `Zeroizing`) le sont. Chaque usage laisse donc brièvement une copie de la clé de session dans un tampon libéré du tas WinRT. Le texte chiffré reste protégé par l'AEAD et la clé enveloppée n'est lisible que par l'utilisateur Windows courant (portée moins étroite que `CRYPTPROTECTMEMORY_SAME_PROCESS`). |
