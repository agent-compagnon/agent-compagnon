---
id: "004"
titre: "core — moteur de politiques, profils et durcissement"
etape: "2c"
statut: "en cours"
depend_de: ["001"]
suivi: "github#5"
---

# 004 — core — moteur de politiques, profils et durcissement

## Objectif

Décider, pour chaque appel d'outil, s'il est autorisé, soumis à confirmation ou interdit, de façon déterministe,
expliquable et testée à fond. Avec le filtre, c'est le cœur de la sécurité.

## Modèle

- `Decision` : `Allow`, `Confirm(channel)`, `Deny`.
- `Channel` : `Elicitation`, `Notification`.
- Sévérité, du plus souple au plus strict : `Allow < Confirm(Elicitation) < Confirm(Notification) < Deny`.
- `Category` : `Read`, `Write`, `Destructive`.
- Niveaux : racine → environnement → MCP → outil.
- Une règle = `{ niveau, sélecteur, decision, locked }`. Sélecteurs : `default`, une catégorie, ou la règle d'outil
  (`tool.<nom>.rule`, qui vaut quel que soit la catégorie).
- Trois arbres de même forme : **base** (`[policy]`), **profil** (`[profiles.<id>.policy]`), **utilisateur**
  (durcissements, dans les réglages locaux).
- Les types vivent dans `core::config` (`Decision`, `Channel`, `Category`, `Rule`, `RuleSet`, `PolicyTree`) et sont
  réexportés par `core::policy`. La position d'une règle (`RulePosition` : environnement, MCP, `Selector`) y est
  définie une seule fois : `applies_to` (la règle est-elle candidate pour un appel) sert au moteur,
  `covers` (une règle verrouillée est-elle candidate chaque fois qu'une autre l'est) sert à la validation (I1.21).

## Résolution

Entrées : chemin (environnement, MCP, outil), catégorie `c`, outil connu ou non.

1. **Candidates** : toutes les règles de la base et du profil posées sur la racine, l'environnement, le MCP ou
   l'outil, dont le sélecteur convient (`default` convient toujours ; une catégorie convient si c'est `c` ; la règle
   d'outil convient pour cet outil).
2. **Gagnante** : la candidate la plus précise, par ordre lexicographique :
   1. sélecteur : règle d'outil > catégorie > défaut ;
   2. profondeur : outil > MCP > environnement > racine ;
   3. couche : profil > base.
3. **Plancher** : la plus sévère des candidates marquées `locked`, toutes couches et tous niveaux confondus ; à
   sévérité égale, la plus précise selon l'ordre du point 2 (pour une provenance déterministe).
4. **Décision de la config** = `max(gagnante, plancher)`.
5. **Outil inconnu** de la config : appliquer 1 à 4 avec `c = Destructive`, puis
   `décision = max(décision, unknown_tools du profil)`, où `unknown_tools` vaut `Confirm(Notification)` ou `Deny`.
   Est inconnu un outil absent de `[mcp.tools]` de son MCP, et aussi tout outil d'un MCP inconnu ou sans instance
   dans cet environnement.
6. **Durcissement utilisateur** : appliquer 1 et 2 sur l'arbre utilisateur seul ; s'il donne un résultat,
   `décision finale = max(décision, résultat)`. L'utilisateur ne peut donc que durcir, y compris au-dessus d'une
   règle verrouillée. `locked` n'a pas de sens dans l'arbre utilisateur (le fichier de réglages ne le connaît pas ;
   un arbre construit en mémoire avec `locked = true` n'a aucun effet de plancher). Le point 2 s'appliquant à
   l'arbre utilisateur seul, une règle utilisateur plus précise et plus souple remplace une règle utilisateur plus
   large ; elle ne descend jamais sous la décision de la config.

La résolution est totale : la validation de la config impose une règle racine `default`. Un arbre construit sans
elle (hors validation) et sans aucune candidate donne `Deny` (échec fermé, INV-5).

API : `PolicyEngine::new(&Config, &Profile, &PolicyTree)` puis
`resolve(env, mcp, outil, annotations: Option<&ToolAnnotations>) -> Resolution`.

Exemples à couvrir tels quels :

| Config | Outil | Résultat |
|---|---|---|
| racine `destructive = deny` ; MCP `default = allow` | destructif, sans règle propre | `Deny` (une règle de catégorie l'emporte sur un défaut plus profond) |
| même config + MCP `destructive = confirm` | destructif | `Confirm` (catégorie plus profonde) |
| env `write = confirm(notification), locked` ; outil `rule = allow` | écriture | `Confirm(Notification)` (le verrou est un plancher) |
| env `write = confirm(notification), locked` ; outil `rule = deny` | écriture | `Deny` (on peut durcir sous un verrou) |
| base `read = allow` ; utilisateur `env.prod.default = confirm(notification)` | lecture en PROD | `Confirm(Notification)` |

Les exemples 3 et 4 portent sur le moteur : dans une config, la règle d'outil `allow` de l'exemple 3 est refusée
au chargement, puisqu'elle est couverte par un verrou plus strict de la même couche (I1.21). Le plancher reste
appliqué par le moteur, et il est seul à agir quand la validation ne peut pas refuser la règle gagnante : verrou
plus profond que la gagnante mais de sélecteur moins précis (par exemple MCP `default` verrouillé et racine
`read = allow`), verrou d'un profil au-dessus d'une règle de la base, catégorie connue seulement à l'exécution.

## Catégorie d'un outil

1. `category` déclarée dans la config ;
2. sinon, déduite des annotations du serveur (`ToolAnnotations { read_only_hint, destructive_hint }`) :
   `readOnlyHint = true` → `Read` ; sinon `destructiveHint = false` → `Write` ; sinon `Destructive` ;
3. sinon `Destructive`. Des annotations sans aucun des deux indices comptent comme absentes (source `Default`).

Un outil inconnu de la config est toujours traité comme `Destructive`, quelles que soient ses annotations. La
source retenue (`Config`, `Annotation`, `Default`, `Unknown`) est conservée.

## Provenance

`resolve` renvoie la décision **et** son explication : règle gagnante (couche, niveau, sélecteur), règle de
plancher éventuelle, durcissement utilisateur éventuel, catégorie et sa source. Cette explication alimente le
journal et l'infobulle « politique » de l'onglet MCP. Prévoir un rendu texte court en français, par exemple
`confirmation (notification) — règle « write » de PROD, verrouillée`.

- `Resolution { decision, category, category_source, provenance }` ;
  `Provenance { winner, floor, unknown_tools, hardening }`, chaque règle en `RuleOrigin { layer, position, rule }`
  avec `Layer` = `Base`, `Profile(id)` ou `User`. `floor` est la plus sévère des candidates verrouillées, qu'elle
  ait relevé la gagnante ou non.
- `Provenance::decided_by()` désigne l'étape qui a fixé la décision : la gagnante, puis le plancher, `unknown_tools`
  et le durcissement, chacun ne comptant que s'il est **strictement** plus sévère que l'étape précédente ; `NoRule`
  sans aucune candidate.
- Rendu : `Resolution::describe(&Config)` donne `<décision> — <ce qui a décidé>` avec les libellés de la config
  (`règle « write » de PROD, verrouillée`, `règle de l'outil « delete_index » de Kibana Logs en PROD`,
  `règle « default » de la racine, profil « Lecture seule »`, `durcissement utilisateur « default » de PROD`,
  `outil inconnu de la config, profil « Développeur »`) ; `describe_detailed` y ajoute la règle la plus précise et
  le plancher quand ils n'ont pas décidé, puis la catégorie et sa source.

## Profils et réglages

- Profils issus de la config : `label`, `default_environments`, `unknown_tools`, `strictest`, arbre de règles.
- `core::settings::UserSettings` : profil choisi (absent au premier lancement), environnements exposés, MCP coupés,
  arbre de durcissement, options (`autostart`, `beta_channel`, `gateway_suspended`), `schema_version`.
- Trait `SettingsStore { load, save }` et une implémentation fichier JSON simple (chemin injecté). Le scellement
  vient en 5d.
- Tant qu'aucun profil n'est choisi : profil `strictest` et aucun environnement exposé.
- `PolicyEngine::summary(env, mcp)` : politique « par défaut » d'un MCP à afficher dans le tableau (décision pour
  chaque catégorie). C'est la décision pour un outil déclaré de cette catégorie sans règle d'outil, durcissement
  utilisateur compris, avec sa provenance.

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
- **EX-004-09** — Cohérence avec la validation, sur la config embarquée : pour chaque profil, chaque instance et chaque outil
  déclaré (sous chaque catégorie d'annotations), la décision n'est jamais plus souple qu'une règle verrouillée
  candidate ; quand le plancher relève la gagnante, c'est une règle que la validation (I1.21) ne pouvait pas
  refuser ; une règle que seul le plancher corrigerait dans la même couche est refusée au chargement.
- **EX-004-10** — `summary` : une décision par catégorie, durcissement compris, règles d'outil ignorées.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Exécution des confirmations (3c et 5c). Scellement des réglages (5d).

## Amendements

- **2026-11-22** — Implémentation (I004.1 à I004.9). Écarts par rapport au texte initial, reportés ci-dessus ; la
  sémantique de la résolution (D5, D6) est inchangée :
  - `Decision::Confirm(channel)` (type existant de l'étape 1) au lieu de `Confirm { channel }` ;
  - précisions de déterminisme et d'échec fermé : plancher à sévérité égale départagé par la précision, `Deny` sans
    aucune candidate, `decided_by` ne retient une étape que si elle est strictement plus sévère ;
  - est aussi inconnu l'outil d'un MCP sans instance dans l'environnement ; des annotations sans indice comptent
    comme absentes ;
  - EX-004-03 se lit par rapport à la décision sans durcissement : le point 6 applique l'ordre de précision à
    l'arbre utilisateur, donc une règle utilisateur plus précise et plus souple remplace une règle utilisateur plus
    large (test `a_more_precise_user_rule_replaces_a_broader_one`), sans jamais descendre sous la config ;
  - les exemples 3 et 4 ne s'écrivent pas tels quels dans une config valide (I1.21) : ils sont testés sur des
    arbres construits en mémoire ;
  - critères ajoutés : `EX-004-09` (cohérence avec la validation), `EX-004-10` (`summary`).
