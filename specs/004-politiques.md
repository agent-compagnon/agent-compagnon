---
id: "004"
titre: "core — moteur de politiques, profils et durcissement"
etape: "2c"
statut: "prévue"
depend_de: ["001"]
suivi: "github#5"
---

# 004 — core — moteur de politiques, profils et durcissement

## Objectif

Décider, pour chaque appel d'outil, s'il est autorisé, soumis à confirmation ou interdit, de façon déterministe,
expliquable et testée à fond. Avec le filtre, c'est le cœur de la sécurité.

## Modèle

- `Decision` : `Allow`, `Confirm { channel }`, `Deny`.
- `Channel` : `Elicitation`, `Notification`.
- Sévérité, du plus souple au plus strict : `Allow < Confirm(Elicitation) < Confirm(Notification) < Deny`.
- `Category` : `Read`, `Write`, `Destructive`.
- Niveaux : racine → environnement → MCP → outil.
- Une règle = `{ niveau, sélecteur, decision, locked }`. Sélecteurs : `default`, une catégorie, ou la règle d'outil
  (`tool.<nom>.rule`, qui vaut quel que soit la catégorie).
- Trois arbres de même forme : **base** (`[policy]`), **profil** (`[profiles.<id>.policy]`), **utilisateur**
  (durcissements, dans les réglages locaux).

## Résolution

Entrées : chemin (environnement, MCP, outil), catégorie `c`, outil connu ou non.

1. **Candidates** : toutes les règles de la base et du profil posées sur la racine, l'environnement, le MCP ou
   l'outil, dont le sélecteur convient (`default` convient toujours ; une catégorie convient si c'est `c` ; la règle
   d'outil convient pour cet outil).
2. **Gagnante** : la candidate la plus précise, par ordre lexicographique :
   1. sélecteur : règle d'outil > catégorie > défaut ;
   2. profondeur : outil > MCP > environnement > racine ;
   3. couche : profil > base.
3. **Plancher** : la plus sévère des candidates marquées `locked`, toutes couches et tous niveaux confondus.
4. **Décision de la config** = `max(gagnante, plancher)`.
5. **Outil inconnu** de la config : appliquer 1 à 4 avec `c = Destructive`, puis
   `décision = max(décision, unknown_tools du profil)`, où `unknown_tools` vaut `Confirm(Notification)` ou `Deny`.
6. **Durcissement utilisateur** : appliquer 1 et 2 sur l'arbre utilisateur seul ; s'il donne un résultat,
   `décision finale = max(décision, résultat)`. L'utilisateur ne peut donc que durcir, y compris au-dessus d'une
   règle verrouillée.

La résolution est totale : la validation de la config impose une règle racine `default`.

Exemples à couvrir tels quels :

| Config | Outil | Résultat |
|---|---|---|
| racine `destructive = deny` ; MCP `default = allow` | destructif, sans règle propre | `Deny` (une règle de catégorie l'emporte sur un défaut plus profond) |
| même config + MCP `destructive = confirm` | destructif | `Confirm` (catégorie plus profonde) |
| env `write = confirm(notification), locked` ; outil `rule = allow` | écriture | `Confirm(Notification)` (le verrou est un plancher) |
| env `write = confirm(notification), locked` ; outil `rule = deny` | écriture | `Deny` (on peut durcir sous un verrou) |
| base `read = allow` ; utilisateur `env.prod.default = confirm(notification)` | lecture en PROD | `Confirm(Notification)` |

## Catégorie d'un outil

1. `category` déclarée dans la config ;
2. sinon, déduite des annotations du serveur : `readOnlyHint = true` → `Read` ; sinon `destructiveHint = false` →
   `Write` ; sinon `Destructive` ;
3. sinon `Destructive`.

Un outil inconnu de la config est toujours traité comme `Destructive`, quelles que soient ses annotations. La
source retenue (`Config`, `Annotation`, `Default`, `Unknown`) est conservée.

## Provenance

`resolve` renvoie la décision **et** son explication : règle gagnante (couche, niveau, sélecteur), règle de
plancher éventuelle, durcissement utilisateur éventuel, catégorie et sa source. Cette explication alimente le
journal et l'infobulle « politique » de l'onglet MCP. Prévoir un rendu texte court en français, par exemple
`confirmation (notification) — règle « write » de PROD, verrouillée`.

## Profils et réglages

- Profils issus de la config : `label`, `default_environments`, `unknown_tools`, `strictest`, arbre de règles.
- `core::settings::UserSettings` : profil choisi (absent au premier lancement), environnements exposés, MCP coupés,
  arbre de durcissement, options (`autostart`, `beta_channel`, `gateway_suspended`), `schema_version`.
- Trait `SettingsStore { load, save }` et une implémentation fichier JSON simple (chemin injecté). Le scellement
  vient en 5d.
- Tant qu'aucun profil n'est choisi : profil `strictest` et aucun environnement exposé.
- `PolicyEngine::summary(env, mcp)` : politique « par défaut » d'un MCP à afficher dans le tableau (décision pour
  chaque catégorie).

## Tests

- **EX-004-01** — Table de cas couvrant chaque branche : sélecteurs, profondeurs, couches, verrous à chaque niveau, outil inconnu
  sous chaque profil, sources de catégorie.
- **EX-004-02** — Les 5 exemples ci-dessus.
- Propriétés (proptest, arbres de règles aléatoires) :
  - **EX-004-03** — ajouter une règle utilisateur ne baisse jamais la sévérité ;
  - **EX-004-04** — une règle verrouillée est une borne inférieure pour tout outil qu'elle couvre, quelles que soient les autres
    règles ;
  - **EX-004-05** — la résolution est totale et déterministe ;
  - **EX-004-06** — un outil inconnu n'est jamais plus souple que le même chemin en `Destructive`.
- **EX-004-07** — La provenance désigne bien la règle qui a décidé, dans chaque cas de la table.
- **EX-004-08** — `SettingsStore` : aller-retour, fichier absent, fichier illisible → réglages par défaut les plus stricts.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Exécution des confirmations (3c et 5c). Scellement des réglages (5d).
