---
id: "030"
titre: "Traçabilité des specs : identifiants de critères et `cargo xtask trace`"
statut: "implémentée"
depend_de: ["001"]
suivi: "github#31"
---

# 030 — Traçabilité des specs : identifiants de critères et `cargo xtask trace`

## Objectif

Empêcher les specs et le code de diverger sans que personne le voie, et garder la mémoire du projet dans le dépôt
plutôt que dans un gestionnaire de tickets.

## Comportement attendu

### Format d'une spec

- Fichier `specs/NNN-nom.md`. En-tête entre deux lignes `---` : `id`, `titre`, `statut`, `depend_de`, et
  facultativement `etape` et `suivi`. Valeurs entre guillemets ; `depend_de` est une liste d'identifiants de specs.
- `statut` vaut `prévue`, `en cours`, `implémentée`, `proposée`, `backlog` ou `à la demande`.
- Un critère d'acceptation est une puce qui commence par `**EX-NNN-nn**`, suivie de `(manuel)` s'il ne peut être
  vérifié qu'à la main. `NNN` est l'identifiant de la spec qui le contient.
- Un critère retiré reste dans le fichier, barré (`~~…~~`). Son identifiant n'est jamais réutilisé.
- Les puces des blocs de code délimités par trois accents graves ne sont pas lues comme des critères.

### `cargo xtask trace`

- Lit toutes les specs et signale : en-tête absent ou invalide, `id` différent du préfixe du nom de fichier, statut
  inconnu, dépendance vers une spec inexistante, identifiant de critère mal formé, en double, ou portant le numéro
  d'une autre spec.
- Relève les identifiants cités dans le code — fichiers suivis par git (`git ls-files`) sous `crates/`, `app/`,
  `xtask/` et `ui/`, hors dossiers `target`, `node_modules` et `dist` — et dans `docs/recette.md`.
- Dans le code, seule compte la forme de citation : le mot `Exigence`, deux-points, puis un ou plusieurs
  identifiants séparés par des virgules. Un identifiant écrit ailleurs (donnée de test, texte) n'est pas une
  citation. Dans `docs/recette.md`, tout identifiant compte.
- Une citation mal formée est une erreur.
- Un identifiant cité mais absent des specs est une erreur. Un critère barré encore cité est une erreur.
- Spec `implémentée` : chaque critère automatique est cité au moins une fois dans le code, chaque critère
  `(manuel)` au moins une fois dans `docs/recette.md`. Sinon, erreur.
- Spec `en cours` : les critères non couverts sont listés en avertissement, sans faire échouer la commande.
- Autres statuts : rien n'est exigé.
- L'index de `specs/README.md`, entre ses deux marqueurs, est comparé aux en-têtes. Toute différence est une
  erreur ; `cargo xtask trace --write-index` le régénère, sauf si un nom de fichier ou un en-tête de spec est
  invalide. Colonne « Critères » : nombre de critères non retirés ; « Dépend de » : une suite d'au moins quatre
  numéros consécutifs s'écrit `001 à 010`.
- Sortie : une ligne par spec (critères couverts sur total), puis les erreurs avec le fichier et l'identifiant
  concernés. Code de sortie non nul s'il y a au moins une erreur.
- `cargo xtask trace --report` affiche la matrice critère → fichiers qui le citent.
- `cargo xtask ci` lance cette vérification après les tests, sous le nom « traçabilité » : après la passe clippy en
  cible Windows, avant `ui/`, dans le groupe `ci test`.

### Rattrapage de l'existant

- Les tests existants des specs 001 et 002 citent leurs critères, sous la forme `// Exigence : EX-001-02`.
- `docs/recette.md` est créé : une section par spec qui a des critères manuels, avec la liste de ces critères à
  cocher lors d'une recette.

## Tests

- **EX-030-01** — Une spec bien formée est lue : identifiant, titre, statut, dépendances, critères automatiques et
  manuels.
- **EX-030-02** — Chaque défaut de format est signalé avec le fichier concerné : en-tête absent, `id` différent du
  nom de fichier, statut inconnu, dépendance inconnue.
- **EX-030-03** — Identifiant de critère en double, mal formé ou portant le numéro d'une autre spec : erreur.
- **EX-030-04** — Spec `implémentée` dont un critère automatique n'est cité par aucun fichier de code : erreur
  nommant le critère.
- **EX-030-05** — Spec `implémentée` dont un critère manuel est absent de `docs/recette.md` : erreur. Le citer dans
  le code ne suffit pas.
- **EX-030-06** — Spec `en cours` : critères non couverts en avertissement, code de sortie nul.
- **EX-030-07** — Identifiant cité dans le code ou la recette mais absent des specs : erreur nommant le fichier.
- **EX-030-08** — Critère barré : non exigé, et erreur s'il est encore cité.
- **EX-030-09** — Index périmé : erreur ; `--write-index` le régénère, et une seconde vérification passe.
- **EX-030-10** — Sur le dépôt réel, `cargo xtask trace` passe et `cargo xtask ci` l'exécute.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Vérifier qu'un test prouve réellement le critère qu'il cite : cela reste du ressort de la relecture.
