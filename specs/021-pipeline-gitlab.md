---
id: "021"
titre: "Pipeline GitLab de release"
etape: "7c"
statut: "implémentée"
depend_de: ["001", "020"]
suivi: "github#22"
---

# 021 — Pipeline GitLab de release

## Objectif

Sur un tag, construire l'exe, le tester, le signer en ed25519 et le déposer au bon chemin dans Artifactory.

## À faire

### Étapes

`lint` → `test` → `build` → `sign` → `publish`. Les demandes de fusion ne lancent que `lint` et `test`. `build`,
`sign` et `publish` ne tournent que sur les tags protégés.

Réalisation (amendement du 2026-11-22) : étapes `lint`, `test`, `build`, `authenticode`, `sign`, `publish`. Règles
`workflow` (forme documentée de GitLab) : pipeline de demande de fusion pour une demande de fusion, pipeline de
branche sinon (jamais les deux pour une même poussée), pipeline de tag. Les jobs de release portent la règle
`if: $CI_COMMIT_TAG && $CI_COMMIT_REF_PROTECTED == "true"` (et, pour un build, le mode choisi), n'utilisent pas le
cache (un pipeline de demande de fusion ne doit rien pouvoir glisser dans un build de release), et l'artefact passe
de `build` à `sign` puis de `sign` (`dependencies: [sign]`) à `publish`.

### Build Windows : deux variantes

Le runner disponible n'est pas connu (D10). La variable `WINDOWS_BUILD_MODE` choisit :

| Valeur | Runner | Build | Tests sous Windows |
|---|---|---|---|
| `windows-runner` (défaut) | Windows avec Build Tools MSVC, tag à ajuster | `cargo build --release -p mcp-proxy --target x86_64-pc-windows-msvc` | oui : `cargo test --workspace` |
| `linux-xwin` | Linux, image Docker avec Rust, clang, lld et cargo-xwin | `cargo xwin build --release …` | non |

- Dans les deux cas : `npm ci` et `npm run build` dans `ui/` avant le build Rust.
- `linux-xwin` a besoin du SDK Microsoft : prévoir une image qui l'embarque déjà, ou l'accès au téléchargement.
  Le documenter.
- Tags de runner et image sont des variables en tête de fichier, commentées.

Réalisation (amendement du 2026-11-22) :

- Variables en tête : `WINDOWS_BUILD_MODE`, `WINDOWS_RUNNER_TAG`, `RUST_IMAGE` (jobs Linux), `XWIN_IMAGE` (valeur
  d'exemple à remplacer), `JFROG_CLI_IMAGE`.
- `windows-runner` : jobs `test:windows` (étape `test`, tags protégés seulement : une demande de fusion n'exige pas
  de runner Windows) et `build:windows`, en PowerShell ; chaque commande native est suivie de
  `if (-not $?) { exit 1 }`, pour qu'un échec arrête le job quel que soit le comportement du runner ; `npm.cmd`
  plutôt que `npm` (le script `npm.ps1` dépend de la stratégie d'exécution).
- `linux-xwin` : job `build:linux-xwin`, en shell POSIX, image `XWIN_IMAGE` ; `docs/release.md` donne un exemple
  d'image (non essayé) qui embarque la bibliothèque d'exécution MSVC et le SDK.
- L'exe va dans `dist/mcp-proxy.exe` (artefact, 7 jours).

### Version

- Le tag `vX.Y.Z` (ou `vX.Y.Z-rc.N` pour le canal bêta) doit être égal à la version du workspace. Sinon le job
  échoue.

Réalisation (amendement du 2026-11-22) : `cargo xtask check-tag <tag>`, première commande de chaque job de build.
La version du workspace est celle de xtask (`CARGO_PKG_VERSION`, `version.workspace = true`) ; seules `X.Y.Z` et
`X.Y.Z-rc.N` (nombres sans zéro initial) sont publiables, toute autre préversion ou métadonnée de build est refusée.

### Signature

- `cargo xtask check-release` : refuse la clé de développement.
- `cargo xtask sign` avec la clé privée dans `ED25519_PRIVATE_KEY`, variable **protégée et masquée**, disponible
  uniquement sur les tags protégés. `RELEASE_CRITICAL` (défaut `false`) renseigne le champ `critical`.
- `cargo xtask verify` juste après, avec la clé publique embarquée.

Réalisation (amendement du 2026-11-22) : job `sign` sous Linux (`RUST_IMAGE`, `MCP_PROXY_RELEASE=1`), sur l'exe des
étapes précédentes : `check-release --exe`, `sign --version "$CI_COMMIT_TAG" --critical "$RELEASE_CRITICAL"
--key-env ED25519_PRIVATE_KEY` (la clé est lue par son nom, jamais développée sur une ligne de commande), `verify`.
Artefacts : l'exe et son `.sig`, 30 jours. `RELEASE_CRITICAL` vaut `"false"` dans le fichier et se donne pour un seul
pipeline à la poussée du tag (`git push … -o ci.variable="RELEASE_CRITICAL=true"`). La clé privée et le jeton
Artifactory sont en plus limités à l'environnement `release`, que seuls `sign` (`action: prepare`) et `publish`
déclarent : `lint`, `test` et `build`, qui exécutent du code tiers, ne les reçoivent pas.

### Emplacement réservé pour Authenticode

- Un job `authenticode`, désactivé, placé **entre `build` et `sign`** : la signature Authenticode modifie l'exe,
  elle doit donc précéder le calcul de l'empreinte. Commentaire expliquant comment l'activer
  (`028-authenticode.md`).

Réalisation (amendement du 2026-11-22) : job caché `.authenticode` (un nom commençant par un point ne tourne jamais)
dans une étape `authenticode` déclarée entre `build` et `sign`. Pour l'activer : le renommer, adapter la commande
`signtool` et ses variables, et ajouter `dependencies: [authenticode]` à `sign`, pour que `sign` prenne l'exe signé
Authenticode et non celui de `build`.

### Publication

- JFrog CLI :
  ```
  jf rt upload mcp-proxy.exe     "$ARTIFACTORY_REPO/mcp-proxy/$VERSION/mcp-proxy.exe"
  jf rt upload mcp-proxy.exe.sig "$ARTIFACTORY_REPO/mcp-proxy/$VERSION/mcp-proxy.exe.sig"
  ```
- Échec si le dossier de la version existe déjà : le dépôt est en release, rien ne s'écrase.
- Identifiants Artifactory en variables protégées et masquées.

Réalisation (amendement du 2026-11-22) :

- Variables : `JF_URL` (plateforme JFrog) et `JF_ACCESS_TOKEN` (jeton de dépôt, masqué), que la JFrog CLI lit
  d'elle-même, sans le jeton sur une ligne de commande ; `ARTIFACTORY_REPO`. Toutes protégées, portée `release`.
- Contrôle d'existence **avant** tout envoi, par la même requête anonyme que l'updater (API Storage,
  `$JF_URL/artifactory/api/storage/…/mcp-proxy/<version>`) : seul un 404 permet de continuer ; 200 ou toute autre
  réponse arrête le job sans rien envoyer.
- L'exe d'abord, le `.sig` ensuite : tant que le `.sig` manque, l'updater ignore le dossier (il lit le `.sig` avant
  l'exe). Deux envois `jf rt upload` ne peuvent pas être atomiques ; si le second échoue, l'exe reste seul, la
  relance échoue au contrôle d'existence, et `docs/release.md` dit comment finir à la main. La garantie que rien ne
  s'écrase est la permission du jeton (dépôt sans suppression ni écrasement), le contrôle n'étant qu'un premier
  filet.
- Après l'envoi : téléchargement anonyme des deux fichiers et comparaison octet à octet avec ceux de `sign`.
- `resource_group: release` : deux publications ne tournent jamais en même temps.

### Documentation

`docs/release.md` : génération de la paire de clés, variables à créer, tags de runner, comment publier une version,
comment publier une version critique, procédure de rotation de clé (publier d'abord une version qui embarque la
nouvelle clé publique dans le second emplacement, puis signer avec la nouvelle clé).

Réalisation (amendement du 2026-11-22) : `docs/release.md` couvre aussi la protection des tags `v*`, la préparation
d'Artifactory, les deux modes de runner, la préversion (canal bêta), la fuite de clé, chaque échec connu de chaque
job, et la liste de toutes les variables du pipeline.

### Contrôle sans GitLab

Le pipeline ne peut pas être exécuté depuis l'environnement de développement. `cargo xtask check-ci` vérifie donc
ce qui peut l'être : le YAML est valide, chaque commande `cargo xtask …` citée existe, les variables utilisées sont
documentées dans `docs/release.md`.

Réalisation (amendement du 2026-11-22) : exécutable `check-ci` du crate `mcp-proxy-ci-check` (lecture du YAML par
`yaml-rust2`), lancé par `cargo xtask check-ci` (xtask reste sans dépendance) et compris dans `cargo xtask ci`
(groupe `test`, donc aussi dans le job `test` des demandes de fusion). Les formes de `cargo xtask` sont dans
`xtask/src/commands.rs`, compilé dans les deux crates. Vérifie aussi :

- la structure : un seul document, une table, aucune clé en double, aucun alias inconnu, aucun `include` ; étape de
  chaque job déclarée, cibles d'`extends`, `dependencies` (étape antérieure) et `needs` existantes, `extends` sans
  cycle, un `script` par job ;
- les jobs des étapes `build`, `authenticode`, `sign`, `publish`, cachés compris : `rules` obligatoire, sans
  `only`/`except`, chaque règle autre que `when: never` exige un tag protégé (conjonction sans `||` de
  `$CI_COMMIT_TAG` et de `$CI_COMMIT_REF_PROTECTED == "true"`) ;
- aucune trace des commandes (`set -x`, `set -o xtrace`, `Set-PSDebug`, `CI_DEBUG_TRACE`, `CI_DEBUG_SERVICES`) ;
- une variable est documentée quand son nom figure entre accents graves dans `docs/release.md` ; sont vérifiées
  toutes les références (`$NOM`, `${NOM…}`, `$env:NOM`) et toutes les clés des tables `variables`.

Le respect des règles de GitLab elles-mêmes (mots-clés, syntaxe de `rules:if`, runners) ne se vérifie qu'au premier
pipeline (`docs/recette.md`).

## Tests

- **EX-021-01** — `check-ci` : succès sur le fichier livré ; échec sur un YAML invalide, une commande xtask inconnue, une variable
  non documentée.
- **EX-021-02** — Concordance tag / version : égal, différent, préversion.
- **EX-021-07** — `check-ci` refuse une structure incohérente : étape non déclarée, `extends`, `dependencies` ou `needs` vers
  un job inconnu (ou d'une étape non antérieure pour `dependencies`), `extends` circulaire, job sans `script`, liste
  des étapes absente, `include`.
- **EX-021-08** — `check-ci` refuse un job de release (étapes `build`, `authenticode`, `sign`, `publish`) qui pourrait tourner
  ailleurs que sur un tag protégé (sans `rules`, avec `only`/`except`, règle sans `$CI_COMMIT_TAG` et
  `$CI_COMMIT_REF_PROTECTED == "true"` ou avec `||`), y compris sur le fichier livré affaibli, et toute trace des
  commandes (`set -x`, `Set-PSDebug`, `CI_DEBUG_TRACE`).
- **EX-021-09** — Chaque forme de `cargo xtask` connue de `check-ci` est acceptée par xtask ; une commande inconnue est
  refusée.

## Vérification

`cargo xtask ci` vert, `cargo xtask check-ci` vert.

## Recette manuelle (GitLab)

- **EX-021-03** (manuel) — Une demande de fusion lance `lint` et `test`.
- **EX-021-04** (manuel) — Un tag protégé produit `mcp-proxy/<version>/mcp-proxy.exe` et son `.sig` dans Artifactory.
- **EX-021-05** (manuel) — Republier le même tag échoue sans rien écraser.
- **EX-021-06** (manuel) — L'exe publié est accepté par l'updater d'une version précédente.
- **EX-021-10** (manuel) — Ni la clé privée ni le jeton Artifactory n'apparaissent dans un journal de job ; seuls `sign` et
  `publish` les reçoivent.
- **EX-021-11** (manuel) — Un tag poussé avec `RELEASE_CRITICAL=true` publie un `.sig` `critical: true` ; un tag
  `vX.Y.Z-rc.N` publie une préversion, proposée au seul canal bêta.
- **EX-021-12** (manuel) — Un tag protégé qui ne nomme pas la version du workspace fait échouer `build` avant toute
  compilation, et rien n'est publié.

## Amendements

- **2026-11-22** — Réalisation : étape `authenticode` déclarée pour le job réservé ; job `test:windows` à part ;
  règles `workflow` ; commande `cargo xtask check-tag` ; jobs de release sans cache ; clé privée et jeton limités à
  l'environnement `release` ; noms des variables Artifactory (`JF_URL`, `JF_ACCESS_TOKEN`, `ARTIFACTORY_REPO`) ;
  contrôle d'existence par l'API Storage anonyme, vérification des fichiers publiés et `resource_group` ;
  `RELEASE_CRITICAL` donné à la poussée du tag. `check-ci` vérifie aussi la structure, la restriction des jobs de
  release aux tags protégés et l'absence de trace des commandes, et fait partie de `cargo xtask ci` : critères
  `EX-021-07` à `EX-021-09` ajoutés. Critères manuels `EX-021-10` à `EX-021-12` ajoutés : la confidentialité des
  secrets dans un vrai pipeline, la version critique et la préversion, et l'échec sur un tag faux ne se prouvent
  qu'à l'exécution. Raisons : `DECISIONS.md`, section « Spec 021 ».
