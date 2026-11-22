---
id: "005"
titre: "gateway — registre d'outils, préfixage, exposition, catalogue embarqué + cache"
etape: "3a"
statut: "prévue"
depend_de: ["001", "004"]
suivi: "github#6"
---

# 005 — gateway — registre d'outils, préfixage, exposition, catalogue embarqué + cache

## Objectif

Savoir quels outils existent, sous quel nom les exposer, et lesquels exposer, **avant même toute connexion**.

## À faire

### Modèle

- `InstanceId { env, mcp }` : un MCP dans un environnement. Une instance n'existe que pour les environnements listés
  dans `[mcp.instances]`. Pas de fusion entre environnements : leurs versions peuvent différer.
- `ToolEntry { instance, original_name, exposed_name, title, description, input_schema, output_schema, category,
  category_source, known, fingerprint, origin }` avec `origin ∈ { Embedded, Cache, Live }`.
- `known` = l'outil est déclaré dans `[mcp.tools]` de la config.

### Nom, description, annotations

- Nom exposé : `{env}_{alias}_{outil}` en minuscules ; tout caractère hors `[a-z0-9_-]` devient `_`. Au-delà de
  `tool_name_max_len` (64), tronquer puis ajouter `_` et 6 caractères hexadécimaux du SHA-256 du nom complet.
  Table de correspondance inverse. Une collision est une erreur.
- Titre : `PROD · Kibana Logs · search_logs`.
- Description : `[PROD — Production] ` suivi de la description d'origine. L'environnement est donc toujours rappelé.
- Annotations exposées au client **recalculées depuis la catégorie retenue**, jamais recopiées du serveur :
  `Read` → `readOnlyHint = true` ; `Write` → `readOnlyHint = false`, `destructiveHint = false` ;
  `Destructive` ou outil inconnu → `destructiveHint = true`.
- Schémas d'entrée et de sortie relayés tels quels, après passage par le filtre.

### Exposition

Un outil est exposé si, et seulement si : un profil est choisi, son environnement est activé (réglages
utilisateur, initialisés depuis `default_environments` du profil), et son MCP n'est pas coupé. La suspension de la
passerelle ne change pas la liste : elle fait refuser les appels (5e).

- `exposed_tools()`, `exposed_count()`, `over_threshold()` (seuil `exposed_tools_warning_threshold`).
- `resolve(exposed_name) -> Option<&ToolEntry>` ; un outil non exposé ne se résout pas.

### Catalogue

Ordre de préférence des sources, par instance : **liste réelle** de cette exécution, sinon **cache local**, sinon
**instantané embarqué**.

- Embarqué : `config/catalog/<mcp>.json`, ou `config/catalog/<mcp>.<env>.json` quand un environnement diffère.
  Format : tableau de définitions d'outils (`name`, `title`, `description`, `inputSchema`, `outputSchema`,
  `annotations`). Embarqués à la compilation.
- Cache : trait `CatalogCache { load(&InstanceId), store(&InstanceId, &CachedCatalog) }` et une implémentation
  fichier (dossier injecté). Le contenu, non secret, passe par le filtre avant écriture. Un outil du cache absent de
  la config reste `known = false`.
- `apply_live_list(instance, tools) -> ToolDiff { added, removed, modified }` remplace les outils de l'instance et
  met à jour le cache.
- Empreinte : SHA-256 du JSON canonique (clés triées) de `{ name, description, inputSchema, annotations }` par
  outil ; empreinte d'instance = hachage des empreintes triées.
- Événements (`tokio::sync::broadcast`) : `ExposureChanged`, `ToolsChanged { instance, diff }`.

### xtask

- `cargo xtask snapshot-catalog --url <url> --out config/catalog/<mcp>.json [--header-env NOM]` : interroge un
  serveur MCP et écrit l'instantané. L'en-tête d'authentification vient d'une variable d'environnement et n'est
  jamais affiché.
- Instantanés écrits à la main pour les 5 MCP fictifs de la config d'exemple, cohérents avec les faux serveurs du
  testkit (3b).

## Tests

- **EX-005-01** — Nommage : préfixe, nettoyage, troncature déterministe, collision détectée, correspondance inverse.
- **EX-005-02** — Description et titre contiennent l'environnement ; annotations selon la catégorie ; outil inconnu marqué
  destructif.
- **EX-005-03** — Matrice d'exposition : profil absent, environnement désactivé, MCP coupé, instance inexistante.
- **EX-005-04** — Préférence des sources : embarqué seul, cache présent, liste réelle.
- **EX-005-05** — `apply_live_list` : ajout, suppression, modification de description ou de schéma ; empreinte insensible à l'ordre
  des clés.
- **EX-005-06** — Seuil d'alerte.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Connexions réelles aux serveurs (3b). Rechargement automatique (3d). Scellement du cache (5d).
