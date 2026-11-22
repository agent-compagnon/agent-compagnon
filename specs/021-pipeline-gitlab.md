---
id: "021"
titre: "Pipeline GitLab de release"
etape: "7c"
statut: "en cours"
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

### Version

- Le tag `vX.Y.Z` (ou `vX.Y.Z-rc.N` pour le canal bêta) doit être égal à la version du workspace. Sinon le job
  échoue.

### Signature

- `cargo xtask check-release` : refuse la clé de développement.
- `cargo xtask sign` avec la clé privée dans `ED25519_PRIVATE_KEY`, variable **protégée et masquée**, disponible
  uniquement sur les tags protégés. `RELEASE_CRITICAL` (défaut `false`) renseigne le champ `critical`.
- `cargo xtask verify` juste après, avec la clé publique embarquée.

### Emplacement réservé pour Authenticode

- Un job `authenticode`, désactivé, placé **entre `build` et `sign`** : la signature Authenticode modifie l'exe,
  elle doit donc précéder le calcul de l'empreinte. Commentaire expliquant comment l'activer
  (`028-authenticode.md`).

### Publication

- JFrog CLI :
  ```
  jf rt upload mcp-proxy.exe     "$ARTIFACTORY_REPO/mcp-proxy/$VERSION/mcp-proxy.exe"
  jf rt upload mcp-proxy.exe.sig "$ARTIFACTORY_REPO/mcp-proxy/$VERSION/mcp-proxy.exe.sig"
  ```
- Échec si le dossier de la version existe déjà : le dépôt est en release, rien ne s'écrase.
- Identifiants Artifactory en variables protégées et masquées.

### Documentation

`docs/release.md` : génération de la paire de clés, variables à créer, tags de runner, comment publier une version,
comment publier une version critique, procédure de rotation de clé (publier d'abord une version qui embarque la
nouvelle clé publique dans le second emplacement, puis signer avec la nouvelle clé).

### Contrôle sans GitLab

Le pipeline ne peut pas être exécuté depuis l'environnement de développement. `cargo xtask check-ci` vérifie donc
ce qui peut l'être : le YAML est valide, chaque commande `cargo xtask …` citée existe, les variables utilisées sont
documentées dans `docs/release.md`.

## Tests

- **EX-021-01** — `check-ci` : succès sur le fichier livré ; échec sur un YAML invalide, une commande xtask inconnue, une variable
  non documentée.
- **EX-021-02** — Concordance tag / version : égal, différent, préversion.

## Vérification

`cargo xtask ci` vert, `cargo xtask check-ci` vert.

## Recette manuelle (GitLab)

- **EX-021-03** (manuel) — Une demande de fusion lance `lint` et `test`.
- **EX-021-04** (manuel) — Un tag protégé produit `mcp-proxy/<version>/mcp-proxy.exe` et son `.sig` dans Artifactory.
- **EX-021-05** (manuel) — Republier le même tag échoue sans rien écraser.
- **EX-021-06** (manuel) — L'exe publié est accepté par l'updater d'une version précédente.
