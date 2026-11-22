# Publier MCP Proxy

Ce document s'adresse à la personne qui publie une version de MCP Proxy, y compris la toute première. Il décrit le
pipeline GitLab de release (spec 021), ce qu'il faut préparer une fois, puis comment publier une version ordinaire,
une version critique et une préversion, comment changer de clé de signature, et quoi faire quand un job échoue.

> **Avant la première publication**, rien ne peut être publié : `crates/deploy/keys/current.pub` est vide dans le
> dépôt, et le build de release échoue sans clé courante. Il faut d'abord dérouler toute la section « Préparer une
> fois », dans l'ordre.

## Ce que fait le pipeline

`.gitlab-ci.yml` contient deux sortes de jobs.

| Étape | Job | Quand | Ce qu'il fait |
|---|---|---|---|
| `lint` | `lint` | toujours | `cargo xtask ci lint` |
| `test` | `test` | toujours | `cargo xtask ci test` (tests, `ui/`, traçabilité, `cargo xtask check-ci`) |
| `test` | `test:windows` | tag protégé, mode `windows-runner` | `cargo test --workspace` sous Windows |
| `build` | `build:windows` ou `build:linux-xwin` | tag protégé, selon `WINDOWS_BUILD_MODE` | `cargo xtask check-tag`, build de `ui/`, build de l'exe avec `MCP_PROXY_RELEASE=1` |
| `authenticode` | `.authenticode` | jamais (réservé, spec 028) | signature Authenticode, désactivée |
| `sign` | `sign` | tag protégé | `cargo xtask check-release --exe`, `cargo xtask sign`, `cargo xtask verify` |
| `publish` | `publish` | tag protégé | dépôt de `mcp-proxy.exe` puis de `mcp-proxy.exe.sig` dans Artifactory |

- Une **demande de fusion** ou une **branche** ne lance que `lint` et `test`. Pour une branche qui a une demande de
  fusion ouverte, seul le pipeline de la demande de fusion tourne (règles `workflow`).
- Un **tag protégé** de la forme `vX.Y.Z` ou `vX.Y.Z-rc.N` lance tout. Un tag non protégé ne lance que `lint` et
  `test`. Un tag protégé d'une autre forme fait échouer `build` (`cargo xtask check-tag`).
- Résultat : `<dépôt>/mcp-proxy/X.Y.Z/mcp-proxy.exe` et `<dépôt>/mcp-proxy/X.Y.Z/mcp-proxy.exe.sig`, que l'updater
  des exe installés découvre (spec 020).
- Les jobs de release n'utilisent pas le cache : un pipeline de demande de fusion ne peut rien glisser dans le build
  d'une version.
- Aucun secret n'est écrit dans `.gitlab-ci.yml` : seulement des noms de variables. Aucun job n'affiche ses commandes
  avec leurs valeurs (`cargo xtask check-ci` refuse `set -x`, `Set-PSDebug` et `CI_DEBUG_TRACE`).

## Préparer une fois

### 1. Créer la paire de clés de signature

Sur un poste de confiance, **hors du dépôt** :

```
cargo xtask keygen --out C:\cles\mcp-proxy-release.key
```

La commande écrit la clé privée (64 caractères hexadécimaux) dans le fichier, qu'elle refuse d'écraser, et affiche
seulement la clé publique et son `key-id` :

```
clé publique : 3f2a…c9 (key-id 1a2b3c4d)
```

- **La clé publique** va sur la ligne non commentée de `crates/deploy/keys/current.pub`, par une demande de fusion
  ordinaire : une clé publique n'est pas un secret, et son historique se relit dans git. Contrôle :
  `cargo xtask check-release` affiche « clé courante : … » et « check-release : OK ».
- **La clé privée** va dans la variable GitLab `ED25519_PRIVATE_KEY` (étape 3), puis dans un coffre hors ligne de
  l'équipe (gestionnaire de mots de passe, coffre-fort). Ensuite, supprimer le fichier du poste.
- Elle ne va **jamais** dans le dépôt, dans un ticket, dans une conversation ni dans un journal.
- Si elle est perdue, plus aucune mise à jour ne sera acceptée par les exe installés : il faudrait réinstaller MCP
  Proxy à la main sur chaque poste. Si elle fuit, voir « Changer de clé de signature ».

### 2. Protéger les tags de version

*Settings > Repository > Protected tags* : protéger `v*`, création permise aux seuls mainteneurs. Sans cela,
`CI_COMMIT_REF_PROTECTED` ne vaut pas `"true"` sur un tag et aucun job de release ne tourne. C'est aussi ce qui
empêche une personne qui peut pousser une branche de déclencher une publication.

### 3. Créer les variables CI/CD

*Settings > CI/CD > Variables*, au niveau du projet. Type « Variable » (pas « File »), « Expand variable reference »
décoché.

| Variable | Valeur | Protégée | Masquée | Portée d'environnement |
|---|---|---|---|---|
| `ED25519_PRIVATE_KEY` | les 64 caractères du fichier de `keygen`, sans espace ni retour à la ligne | oui | oui (« Masked and hidden » si proposé) | `release` |
| `JF_ACCESS_TOKEN` | jeton Artifactory qui peut déposer dans le dépôt, **sans** droit de suppression ni d'écrasement | oui | oui | `release` |
| `JF_URL` | URL de la plateforme JFrog, sans `/artifactory` ni `/` final : `https://artifactory.example.internal` | oui | non | `release` |
| `ARTIFACTORY_REPO` | nom du dépôt générique, le même que `repository` dans `[update]` de `config/default.toml` | oui | non | `release` |

Pourquoi :

- **Protégée** : GitLab ne donne la variable qu'aux pipelines des branches et tags protégés. Une demande de fusion ou
  une branche ne peut pas la lire, même en modifiant `.gitlab-ci.yml`.
- **Masquée** : si la valeur est affichée par erreur, GitLab la remplace par `[MASKED]` dans le journal. C'est un
  filet, pas une garantie (une valeur encodée ou découpée n'est pas reconnue) : le pipeline ne l'affiche jamais.
- **Portée `release`** : seuls les jobs qui déclarent l'environnement `release` (`sign` et `publish`) la reçoivent.
  `lint`, `test` et `build` exécutent du code tiers (paquets npm, scripts de build des crates) qui ne voit donc
  jamais la clé ni le jeton. Si votre instance ne propose pas de portée d'environnement, laisser « All » : les
  variables restent réservées aux tags protégés, mais tous les jobs d'un pipeline de tag les reçoivent ; c'est un
  risque à accepter en connaissance de cause.
- Ne jamais définir `RELEASE_CRITICAL` dans les réglages du projet : toutes les versions deviendraient critiques.
- Ne jamais activer `CI_DEBUG_TRACE` dans ce projet : il affiche toutes les variables.
- Facultatif (selon l'édition de GitLab) : *Settings > CI/CD > Protected environments*, protéger `release` pour
  restreindre qui peut lancer `sign` et `publish`.

`JF_URL` et `JF_ACCESS_TOKEN` sont les variables que la JFrog CLI lit d'elle-même : le jeton ne passe sur aucune
ligne de commande. Si vous n'avez qu'un identifiant et un mot de passe, créer `JF_USER` et `JF_PASSWORD` (protégées,
la seconde masquée, portée `release`) à la place de `JF_ACCESS_TOKEN`, et les ajouter au tableau des variables
ci-dessous, sinon `cargo xtask check-ci` ne les verra pas documentées s'ils apparaissent dans le fichier.

### 4. Préparer Artifactory

- Un dépôt **générique** en release : aucun fichier n'y est modifiable. Lecture **anonyme** (D11) : l'updater et le
  job `publish` lisent sans identifiant.
- Le jeton de `JF_ACCESS_TOKEN` peut déposer (« Deploy ») dans ce dépôt, mais pas supprimer ni écraser (« Delete /
  Overwrite ») : c'est la vraie garantie que rien ne s'écrase, le contrôle du job n'étant qu'un premier filet.
- `config/default.toml`, section `[update]` : `base_url` = `JF_URL` suivi de `/artifactory`, `repository` =
  `ARTIFACTORY_REPO`, `path = "mcp-proxy"`. Le job `publish` vérifie ses fichiers à cette même adresse.

### 5. Choisir et préparer le runner du build Windows

Les variables en tête de `.gitlab-ci.yml` se règlent dans le fichier (ou, pour essayer, comme variables du pipeline).

| Variable | Rôle |
|---|---|
| `WINDOWS_BUILD_MODE` | `windows-runner` (défaut) ou `linux-xwin` |
| `WINDOWS_RUNNER_TAG` | tag du runner Windows |
| `RUST_IMAGE` | image des jobs Linux (`lint`, `test`, `sign`) : `rust:1.97.0` |
| `XWIN_IMAGE` | image du build `linux-xwin` ; la valeur livrée est un exemple à remplacer |
| `JFROG_CLI_IMAGE` | image du job `publish` : JFrog CLI, `curl`, `cmp`, un shell POSIX ; épingler une version testée |

**Mode `windows-runner`** (défaut) : un runner GitLab sous Windows, exécuteur « shell » en PowerShell (`pwsh` ou
`powershell`), qui porte le tag `WINDOWS_RUNNER_TAG`, avec :

- Visual Studio Build Tools, charge de travail C++ (MSVC et SDK Windows) ;
- `rustup` dans le `PATH` du compte du runner (la chaîne 1.97.0 et la cible Windows s'installent d'après
  `rust-toolchain.toml`), `git`, Node 22 et npm (`npm.cmd`) ;
- un compte du runner qui a un profil Windows chargé : `cargo test --workspace` lance les tests DPAPI réels
  (spec 002), qui en ont besoin.

Ce mode lance aussi `test:windows` (tous les tests sous Windows) avant le build. L'image `RUST_IMAGE` est ignorée par
l'exécuteur shell.

**Mode `linux-xwin`** : un runner Docker Linux et l'image `XWIN_IMAGE`, qui doit contenir `rustup`, `clang`, `lld`,
`llvm` (dont `llvm-rc`, pour les ressources Windows de l'exe), `cargo-xwin`, `curl` et `xz`. `cargo-xwin` a besoin de
la bibliothèque d'exécution MSVC et du SDK Windows, qu'il télécharge depuis les serveurs de Microsoft (licence
Microsoft, à accepter par votre organisation) : mieux vaut une image qui les contient déjà, ou bien autoriser ces
téléchargements depuis le runner. Aucun test ne tourne sous Windows dans ce mode. Exemple d'image, **non essayé** :

```Dockerfile
FROM rust:1.97.0
RUN apt-get update \
 && apt-get install -y --no-install-recommends clang lld llvm xz-utils \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-msvc && cargo install --locked cargo-xwin
# Downloads the MSVC CRT and the Windows SDK once, into the image (cache of cargo-xwin under /root/.cache).
RUN cargo new /tmp/warmup \
 && cd /tmp/warmup \
 && cargo xwin build --release --target x86_64-pc-windows-msvc \
 && rm -rf /tmp/warmup
```

### 6. Vérifier sans GitLab

```
cargo xtask check-ci
```

Vérifie ce qui peut l'être sans GitLab : le YAML est valide, les étapes, `extends`, `dependencies` et `needs` se
tiennent, les jobs de release ne tournent que sur un tag protégé, aucune trace de commande n'est activée, chaque
`cargo xtask …` cité existe et chaque variable utilisée ou définie figure dans le tableau « Variables » ci-dessous.
Il fait partie de `cargo xtask ci`. Le comportement réel du pipeline se vérifie au premier tag (`docs/recette.md`,
section 021).

## Publier une version

1. Choisir le numéro `X.Y.Z` : strictement supérieur à la dernière version publiée (l'updater n'installe jamais une
   version inférieure ou égale).
2. Mettre `version = "X.Y.Z"` dans `[workspace.package]` du `Cargo.toml` racine, lancer `cargo xtask ci` (qui met
   aussi `Cargo.lock` à jour), passer par une demande de fusion, fusionner.
3. Sur le commit fusionné de la branche principale :

   ```
   git tag -a vX.Y.Z -m "MCP Proxy X.Y.Z"
   git push origin vX.Y.Z
   ```

4. Suivre le pipeline du tag (*Build > Pipelines*) :
   - `build:…` commence par `check-tag : vX.Y.Z = version X.Y.Z du workspace (canal stable)` ;
   - `sign` affiche `check-release : OK`, puis `dist/mcp-proxy.exe.sig : version X.Y.Z, sha256 …, critical false,
     key-id …`, puis `vérifié : version X.Y.Z, critical false, key-id …` ; le `key-id` est celui de la clé courante ;
   - `publish` se termine par `publié dans <dépôt>/mcp-proxy/X.Y.Z/ (mcp-proxy.exe, mcp-proxy.exe.sig)`.
5. Les postes l'installent à leur vérification suivante (une minute après le démarrage de l'appli, puis toutes les
   `check_interval_hours`).

Les artefacts de `sign` (l'exe et son `.sig`) restent 30 jours dans GitLab.

## Publier une version critique

Une version critique redémarre l'appli toute seule dès qu'aucun appel n'est en cours (spec 020). Le champ `critical`
du `.sig` vient de `RELEASE_CRITICAL`, qui vaut `"false"` par défaut. Il se donne **au moment de pousser le tag**,
pour ce seul pipeline :

```
git push origin vX.Y.Z -o ci.variable="RELEASE_CRITICAL=true"
```

Vérifier dans le journal de `sign` : `critical true`. Toute autre valeur que `true` ou `false` fait échouer `sign`.

Un tag poussé sans l'option lance aussitôt un pipeline ordinaire, qui publie une version **non** critique ; elle ne
peut plus être changée (rien ne s'écrase). Dans ce cas, publier tout de suite `X.Y.(Z+1)` en critique.

## Publier une préversion (canal bêta)

1. `version = "X.Y.Z-rc.N"` dans le `Cargo.toml` racine (seule forme de préversion acceptée : `rc.` suivi d'un
   nombre), demande de fusion, fusion.
2. `git tag -a vX.Y.Z-rc.N -m "MCP Proxy X.Y.Z-rc.N"` puis `git push origin vX.Y.Z-rc.N`.
3. `check-tag` affiche `(préversion, canal bêta)`. La version est publiée sous `mcp-proxy/X.Y.Z-rc.N/` ; seuls les
   postes qui ont coché « Recevoir les préversions » la reçoivent.
4. Ensuite `X.Y.Z-rc.(N+1)`, puis `X.Y.Z`, qui passe devant toutes ses préversions dans l'ordre semver.

Une préversion peut aussi être critique (même option `-o ci.variable`).

## Changer de clé de signature

L'exe embarque deux emplacements de clé publique : courante (`current.pub`) et suivante (`next.pub`). Il accepte une
version signée par l'une ou l'autre. Une rotation se fait donc en deux versions :

1. **Préparer** : `cargo xtask keygen --out C:\cles\mcp-proxy-release-2.key`. Mettre la **nouvelle** clé publique
   dans `crates/deploy/keys/next.pub`, sans toucher à `current.pub`. Publier une version `N` comme d'habitude :
   elle est encore signée par l'**ancienne** clé (`ED25519_PRIVATE_KEY` inchangée) et embarque les deux clés.
2. **Attendre** que tous les postes tournent en version `N` ou plus.
3. **Basculer** : mettre la nouvelle clé publique dans `current.pub` et vider `next.pub` ; remplacer la valeur de
   `ED25519_PRIVATE_KEY` par la nouvelle clé privée (et la ranger au coffre, retirer l'ancienne). Publier `N+1` : elle
   est signée par la nouvelle clé, que les postes en version `N` connaissent déjà. Le `key-id` affiché par `sign`
   change.

Un poste resté avant `N` ne connaît pas la nouvelle clé : il refusera `N+1` et toutes les suivantes (l'updater
n'essaie que la plus haute version), et devra être réinstallé à la main. D'où l'attente de l'étape 2.

Si la clé privée a fuité : quiconque la détient peut signer une mise à jour que les postes accepteront. Retirer
aussitôt la variable de GitLab, et faire la rotation sans attendre ; si une version frauduleuse a pu être publiée,
prévoir la réinstallation des postes.

## Quand une étape échoue

Tant que `publish` n'a rien envoyé, rien n'est publié : corriger, puis relancer le pipeline du tag (ou, si le correctif
change le code, publier un nouveau numéro). Ne jamais supprimer ni déplacer un tag déjà publié.

| Job | Message | Quoi faire |
|---|---|---|
| `build:…` | `check-tag : ÉCHEC : le tag « … » ne correspond pas à la version du workspace` | Le tag ne nomme pas la version du `Cargo.toml`. Supprimer ce tag (rien n'a été publié) et créer le bon, ou monter la version puis retaguer. |
| `build:…` | `aucune clé de mise à jour courante (keys/current.pub)`, ou clé de développement dans un emplacement | La clé publique de release n'est pas dans `current.pub` (étape 1 de la préparation). |
| `build:linux-xwin` | échec de `cargo xwin` au téléchargement | L'image n'embarque pas le SDK et le runner ne peut pas le télécharger : voir « Mode `linux-xwin` ». |
| `test:windows` | tests DPAPI (`platform::windows`) en échec | Le compte du runner n'a pas de profil chargé ; lancer le runner sous un compte utilisateur. |
| `sign` | `check-release : ÉCHEC : l'exe ne contient pas la clé de mise à jour courante` | L'exe ne vient pas de ce commit, ou l'activation d'`authenticode` n'a pas mis `dependencies: [authenticode]`. |
| `sign` | `sign : ÉCHEC : ED25519_PRIVATE_KEY : variable de la clé privée absente ou vide` | Variable absente, non protégée alors que le tag l'est, ou portée différente de `release`. |
| `sign` | `… : clé privée illisible : 64 caractères hexadécimaux minuscules attendus` | Valeur mal collée (espace intérieur, majuscules, autre contenu). |
| `sign` | `sign : … l'exe porte la version …` | L'exe ne porte pas la version du tag : le build ne vient pas du bon commit. |
| `sign` | `verify : ÉCHEC : …` (`key-id` inconnu) | La clé privée ne correspond pas à `current.pub` (ni à `next.pub`). Rien n'est publié : corriger la variable. |
| `sign` | `--critical : true ou false attendu` | `RELEASE_CRITICAL` mal écrit à la poussée du tag. |
| `publish` | `mcp-proxy/X.Y.Z existe déjà … (HTTP 200)` | Cette version est déjà publiée. Rien n'est envoyé. Publier un numéro supérieur. |
| `publish` | `… (HTTP 000)`, `401`, `403`, `5xx` | Artifactory injoignable, ou lecture anonyme refusée (`JF_URL`, `ARTIFACTORY_REPO`, droits). Rien n'est envoyé : corriger puis relancer `publish`. |
| `publish` | échec du **premier** `jf rt upload` (l'exe) | Rien, ou un exe partiel, est arrivé. Si le dossier existe désormais, la relance échouera au contrôle : faire vérifier le dossier par un administrateur d'Artifactory ; sinon relancer `publish`. |
| `publish` | échec du **second** `jf rt upload` (le `.sig`) | L'exe est déposé sans son `.sig` : les postes ignorent ce dossier (aucune signature, rien n'est installé). Relancer `publish` échouerait (le dossier existe). Terminer à la main avec le `.sig` des artefacts de `sign` de ce pipeline, et lui seul : `jf rt upload mcp-proxy.exe.sig "$ARTIFACTORY_REPO/mcp-proxy/X.Y.Z/mcp-proxy.exe.sig"`, puis vérifier comme ci-dessous. |
| `publish` | `cmp` : fichiers différents, ou `curl` en échec après l'envoi | Ce que les postes téléchargent n'est pas ce qui a été signé, ou n'est pas lisible anonymement. Les postes refuseront (empreinte, signature). Faire examiner le dossier ; publier un numéro supérieur. |

Après une reprise à la main, vérifier depuis un poste : `cargo xtask verify --exe mcp-proxy.exe --sig
mcp-proxy.exe.sig` sur les deux fichiers téléchargés.

## Variables

Toutes les variables que `.gitlab-ci.yml` utilise ou définit. `cargo xtask check-ci` refuse le fichier si l'une
d'elles manque ici (nom entre accents graves).

**À créer dans GitLab** (*Settings > CI/CD > Variables*, voir « Créer les variables CI/CD ») : `ED25519_PRIVATE_KEY`,
`JF_ACCESS_TOKEN`, `JF_URL`, `ARTIFACTORY_REPO`.

**Réglées dans `.gitlab-ci.yml`** :

| Variable | Rôle |
|---|---|
| `WINDOWS_BUILD_MODE` | variante du build Windows : `windows-runner` ou `linux-xwin` |
| `WINDOWS_RUNNER_TAG` | tag du runner Windows |
| `RUST_IMAGE` | image des jobs Linux |
| `XWIN_IMAGE` | image du build `linux-xwin` |
| `JFROG_CLI_IMAGE` | image du job `publish` |
| `RELEASE_CRITICAL` | `"false"` ; `"true"` pour une version critique, donné à la poussée du tag |
| `MCP_PROXY_RELEASE` | `"1"` dans les jobs de build et `sign` : le build échoue sans clé courante ou avec la clé de développement (spec 020) |
| `JFROG_CLI_REPORT_USAGE` | `"false"` : la JFrog CLI n'envoie pas de statistiques d'usage |
| `AUTHENTICODE_TIMESTAMP_URL` | réservée au job `.authenticode` (désactivé) : serveur d'horodatage, à créer quand la spec 028 sera faite |
| `CARGO_HOME` | cache de cargo dans le projet, pour le cache des jobs `lint` et `test` |
| `CARGO_TERM_COLOR` | couleurs de cargo dans les journaux |
| `CARGO_INCREMENTAL` | `"0"` : pas de compilation incrémentale en CI |
| `NODE_VERSION` | version de Node installée dans les jobs Linux |
| `npm_config_cache` | cache de npm dans le projet |

**Internes au job `publish`** : `VERSION` (le tag sans son `v`), `HTTP_STATUS` (réponse d'Artifactory au contrôle
d'existence).

**Prédéfinies par GitLab** : `CI_COMMIT_TAG` (le tag du pipeline), `CI_COMMIT_REF_PROTECTED` (`"true"` sur une
référence protégée), `CI_PIPELINE_SOURCE` (`merge_request_event` pour une demande de fusion), `CI_COMMIT_BRANCH`
(la branche d'un pipeline de branche), `CI_OPEN_MERGE_REQUESTS` (demandes de fusion ouvertes de la branche),
`CI_PROJECT_DIR` (dossier du dépôt cloné).
