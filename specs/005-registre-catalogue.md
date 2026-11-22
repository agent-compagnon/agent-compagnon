---
id: "005"
titre: "gateway — registre d'outils, préfixage, exposition, catalogue embarqué + cache"
etape: "3a"
statut: "implémentée"
depend_de: ["001", "004"]
suivi: "github#6"
---

# 005 — gateway — registre d'outils, préfixage, exposition, catalogue embarqué + cache

## Objectif

Savoir quels outils existent, sous quel nom les exposer, et lesquels exposer, **avant même toute connexion**.

## À faire

Tout vit dans `mcp_proxy_gateway::registry`. Aucune dépendance au SDK MCP : le registre a ses propres types de
définition d'outil, que la spec 006 convertit vers et depuis ceux de `rmcp`.

### Modèle

- `InstanceId { env, mcp }` : un MCP dans un environnement. Une instance n'existe que pour les environnements listés
  dans `[mcp.instances]`. Pas de fusion entre environnements : leurs versions peuvent différer.
- `ToolDefinition { name, title, description, input_schema, output_schema, annotations }`, sérialisée comme le
  `Tool` de MCP (`inputSchema`, `outputSchema`…), et `ToolHints { title, read_only_hint, destructive_hint,
  idempotent_hint, open_world_hint }` pour les annotations. C'est le format du catalogue embarqué, du cache, des
  listes reçues des serveurs et des listes envoyées aux clients.
- `ToolEntry { instance, original_name, exposed_name, title, description, input_schema, output_schema, annotations,
  server_annotations, server_title, server_description, category, category_source, known, fingerprint, origin }`
  avec `origin ∈ { Embedded, Cache, Live }` (`CatalogSource`). `exposed_definition()` donne la définition envoyée
  aux clients, `server_definition()` celle du serveur après filtrage (ce que garde le cache).
- `known` = l'outil est déclaré dans `[mcp.tools]` de la config. Un outil vu dans l'instantané, le cache ou la
  liste réelle sans y être déclaré reste inconnu : c'est la config qui décide. Catégorie et source par
  `mcp_proxy_core::policy::tool_category` (S11), la règle du moteur de politiques, indépendante du profil.

### Nom, description, annotations

- Nom exposé : `{env}_{alias}_{outil}` en minuscules ; tout caractère hors `[a-z0-9_-]` devient `_`. Au-delà de
  `tool_name_max_len` (64), tronquer puis ajouter `_` et 6 caractères hexadécimaux du SHA-256 du nom complet
  (`mcp_proxy_core::naming::exposed_tool_name`). Table de correspondance inverse (`lookup`). Une collision est une
  erreur : la liste qui la provoque est refusée en entier.
- Titre : `PROD · Kibana Logs · search_logs`.
- Description : `[PROD — Production] ` suivi de la description d'origine. L'environnement est donc toujours rappelé.
  Sans description d'origine : `[PROD — Production]` seul.
- Annotations exposées au client **recalculées depuis la catégorie retenue**, jamais recopiées du serveur :
  `Read` → `readOnlyHint = true` ; `Write` → `readOnlyHint = false`, `destructiveHint = false` ;
  `Destructive` ou outil inconnu → `readOnlyHint = false`, `destructiveHint = true`. Aucune autre annotation du
  serveur (`idempotentHint`, `openWorldHint`, `title`) n'est relayée.
- Schémas d'entrée et de sortie relayés tels quels, après passage par le filtre. Chaque définition, quelle que soit
  sa source, passe **entière** par le filtre avant d'entrer dans le registre ; un nom qui en ressortirait changé
  (secret connu dans le nom) fait refuser la liste. Un schéma qui n'est pas un objet JSON aussi.

### Exposition

Un outil est exposé si, et seulement si : un profil est choisi, son environnement est activé (réglages
utilisateur, initialisés depuis `default_environments` du profil), et son MCP n'est pas coupé. La suspension de la
passerelle ne change pas la liste : elle fait refuser les appels (5e).

- `Exposure::from_settings(&UserSettings, &Config)` ; `set_exposure` et `apply_settings` envoient
  `ExposureChanged` seulement si l'exposition change. Un profil choisi absent de la config compte comme aucun.
- `exposed_tools()` (triés par nom exposé), `exposed_count()`, `over_threshold()` : strictement plus d'outils
  exposés que `exposed_tools_warning_threshold`.
- `resolve(exposed_name) -> Option<&ToolEntry>` ; un outil non exposé ne se résout pas. `lookup` donne la
  correspondance inverse quelle que soit l'exposition, pour l'interface et le journal, jamais pour servir un appel.

### Catalogue

Ordre de préférence des sources, par instance : **liste réelle** de cette exécution, sinon **cache local**, sinon
**instantané embarqué**.

- Embarqué : `config/catalog/<mcp>.json`, ou `config/catalog/<mcp>.<env>.json` quand un environnement diffère.
  Format : tableau de définitions d'outils (`name`, `title`, `description`, `inputSchema`, `outputSchema`,
  `annotations`). Embarqués à la compilation (`EmbeddedCatalog::builtin`). Un fichier mal nommé, d'un MCP inconnu,
  d'un environnement sans instance, ou illisible est une erreur ; un instantané inutilisable empêche de construire
  le registre.
- Cache : trait `CatalogCache { load(&InstanceId) -> Result<Option<CachedCatalog>>, store(&InstanceId,
  &CachedCatalog) -> Result<()> }` et une implémentation fichier, `FileCatalogCache` (dossier injecté, un fichier
  `<mcp>.<env>.json` par instance, écriture par fichier temporaire puis renommage). Le contenu, non secret, passe
  par le filtre avant écriture ; **si le filtre échoue, rien n'est écrit**. Le fichier porte la version de format et
  l'instance : un fichier d'une autre instance, d'une autre version, illisible ou refusé par le registre est ignoré,
  signalé dans `cache_warnings()`, et l'instantané embarqué est utilisé.
- Scellement (spec 014) : trait `CacheSeal { seal, open }` appliqué aux octets du fichier, avec l'instance pour
  lier un fichier à son instance ; `NoSeal` en attendant. Depuis la spec 014, l'appli utilise `SealedCatalog`.
- `apply_live_list(instance, tools) -> Result<ToolDiff { added, removed, modified }, RegistryError>` remplace les
  outils de l'instance et met à jour le cache. Liste refusée en entier, registre et cache inchangés : instance
  inconnue, nom invalide ou masqué, outil en double, schéma qui n'est pas un objet, collision, filtre en échec. Le
  cache n'est réécrit que si l'empreinte change ou si la liste précédente venait de l'instantané ; un échec
  d'écriture ne refuse pas la liste, il est signalé dans `cache_warnings()`.
- Empreinte : SHA-256 du JSON canonique (clés triées, quel que soit l'ordre reçu) de `{ name, description,
  inputSchema, outputSchema, annotations }` par outil, après filtrage ; empreinte d'instance = hachage des
  empreintes triées.
- Événements (`tokio::sync::broadcast`, `subscribe()`) : `ExposureChanged`, `ToolsChanged { instance, diff }`. Une
  liste de même empreinte n'envoie rien.

### Instantanés

- Instantanés écrits à la main pour les 5 MCP fictifs de la config d'exemple : exactement les outils déclarés, avec
  des annotations pour les outils déclarés sans catégorie. Les faux serveurs du testkit (3b) les reprendront. La
  commande qui les produit depuis un vrai serveur (`cargo xtask snapshot-catalog`) relève de la spec 006.

## Tests

- **EX-005-01** — Nommage : préfixe, nettoyage, troncature déterministe, collision détectée, correspondance inverse.
- **EX-005-02** — Description et titre contiennent l'environnement ; annotations selon la catégorie ; outil inconnu marqué
  destructif.
- **EX-005-03** — Matrice d'exposition : profil absent, environnement désactivé, MCP coupé, instance inexistante.
- **EX-005-04** — Préférence des sources : embarqué seul, cache présent, liste réelle.
- **EX-005-05** — `apply_live_list` : ajout, suppression, modification de description ou de schéma ; empreinte insensible à l'ordre
  des clés.
- **EX-005-06** — Seuil d'alerte.
- **EX-005-07** — Instantanés embarqués : chaque fichier de `config/catalog/` est embarqué, et chaque instance de la config
  d'exemple liste exactement ses outils déclarés, tous connus.
- **EX-005-08** — Filtre : une définition qui contient un secret connu entre masquée dans le registre et dans le cache ; un
  nom masqué fait refuser la liste ; filtre en échec → registre inchangé, aucun fichier de cache écrit, aucun
  registre construit ; le scellement s'applique aux octets du fichier.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Connexions réelles aux serveurs (3b). Rechargement automatique (3d). Scellement du cache (5d).

## Amendements

- **2026-11-22** — `cargo xtask snapshot-catalog` sort de cette spec et passe à la spec 006, section « À faire ».
  Raison : elle interroge un serveur MCP, donc a besoin du client MCP HTTP, qui n'arrive qu'en 006 ; `xtask` doit
  rester sans dépendance lourde. Les instantanés des 5 MCP de la config d'exemple restent ici, écrits à la main.
- **2026-11-22** — Implémentation (I005.1 à I005.13). Écarts par rapport au texte initial, reportés ci-dessus :
  - types propres `ToolDefinition` et `ToolHints`, sans dépendance au SDK MCP ; `ToolEntry` garde aussi les
    annotations exposées et le titre, la description et les annotations du serveur ;
  - `apply_live_list` et les méthodes du cache renvoient un `Result` ; une liste inutilisable est refusée en entier
    et l'instance garde sa liste précédente ; un cache illisible est ignoré et signalé (`cache_warnings`) ;
  - l'empreinte couvre aussi `outputSchema`, et se calcule après filtrage ;
  - toute définition passe par le filtre avant d'entrer dans le registre, pas seulement avant l'écriture du cache ;
  - trait `CacheSeal` pour le scellement de la spec 014 ;
  - `lookup` (correspondance inverse sans tenir compte de l'exposition) et `Exposure` ;
  - critères ajoutés : `EX-005-07` (instantanés embarqués) et `EX-005-08` (filtre et échec fermé).
- **2026-11-22** — Spec 014 : l'appli scelle les fichiers du cache par `SealedCatalog` (même clé que les réglages) ;
  `NoSeal` ne sert plus qu'aux tests. Comportement et critères de la 005 inchangés.
