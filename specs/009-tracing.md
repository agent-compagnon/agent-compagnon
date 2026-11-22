---
id: "009"
titre: "Tracing — spans, tampon circulaire, journal fichier filtré, export de diagnostic"
etape: "3e"
statut: "implémentée"
depend_de: ["003", "007"]
suivi: "github#10"
---

# 009 — Tracing — spans, tampon circulaire, journal fichier filtré, export de diagnostic

## Objectif

Pouvoir comprendre ce qui s'est passé pour chaque appel, sans jamais écrire un secret.

## À faire

### Span par appel

Span `mcp.call` avec : `call_id`, `client`, `env`, `mcp`, `tool`, `session`, `policy`, `status`, `duration_ms`.

Précisions de réalisation : le span est ouvert par `CallChain` (crate gateway) avant le premier maillon et couvre
toute la chaîne, épilogue compris. `call_id`, `client` et `tool` sont posés à l'ouverture, `client` et `tool` après
le filtre de sortie ; les autres champs sont enregistrés à la fin de l'appel : identifiants de la config pour `env`,
`mcp`, `session`, clé de la décision pour `policy` (`allow`, `confirm_elicitation`, `confirm_notification`, `deny`,
`trace::decision_key`), clé du statut pour `status`. Un champ inconnu (outil non résolu) reste vide.

### `CallRecord` et tampon circulaire

- `CallRecord { id, started_at, duration, client, env, mcp, tool, session, policy, status, request, response,
  filter_report, slow }`. `policy` porte la décision et son explication (2c).
- Statuts : `ok`, `error`, `denied` (politique), `refused` (utilisateur), `timeout` (confirmation), `blocked`
  (secret dans les arguments), `rate_limited`, `auth_required`, `leak_masked`.
- Tampon circulaire en mémoire, borné en nombre (`ring_capacity`) et en octets (`ring_max_bytes`).
- Le contenu (requête et réponse) est filtré **avant** d'entrer dans le tampon : il n'y a jamais de contenu brut,
  même en mémoire.
- Implémente le `TraceSink` de 3c.

Précisions de réalisation :

- `CallRecord` (spec 007) n'a pas de champ `slow` : le journal le calcule, `duration ≥ slow_call_threshold_secs`.
  La chaîne émet trois statuts de plus : `suspended` (passerelle suspendue par l'utilisateur), `input_required`
  (confirmation demandée dans le chat, 2026-07-28), `cancelled` (annulé par le client).
- `trace::Journal` implémente `TraceSink`. Le tampon garde des `JournalEntry`, pas des `CallRecord` : chaque texte
  (client, outil, résumé, explication) et chaque valeur (requête, réponse) est **refiltré** par le journal avant
  d'y entrer, quoi qu'ait fait l'appelant ; si le filtre est indisponible, le texte devient `[SECRET MASQUÉ]` et la
  valeur aussi.
- Poids d'une entrée : taille fixe de la structure, plus ses textes, plus la longueur JSON de la requête et de la
  réponse. Les plus anciennes entrées sortent jusqu'à ce que les deux bornes tiennent. Une entrée plus lourde que
  tout le tampon perd ses détails (son résumé l'indique) ; client et outil sont bornés à 256 octets.
- `record` ne fait aucune entrée-sortie et ne prend que le verrou court du tampon : le fichier est écrit par un fil
  dédié (voir plus bas), la diffusion passe par un canal `broadcast` qui ne bloque pas.

### Journal fichier

- Dossier `%LOCALAPPDATA%\MCP Proxy\logs\` (chemin injecté). Rotation quotidienne, rétention `log_retention_days`,
  taille bornée.
- Pour un appel normal : une ligne de résumé, **sans contenu**.
- Le contenu complet d'un appel n'est écrit que s'il est en erreur, bloqué, refusé ou lent (durée ≥
  `slow_call_threshold_secs`, 5 s par défaut).
- Toutes les lignes passent par le `MakeWriter` filtrant de 2b, y compris celles des bibliothèques.

Précisions de réalisation :

- `trace::open_log(LogFileOptions, horloge, filtre)` crée le dossier, supprime les fichiers échus et lance le fil
  d'écriture ; il rend un `FileLog` (seul écrivain disponible : un `FilteringMakeWriter` sur le fichier) et un
  `LogGuard` qui arrête le fil après les lignes en attente. Rien n'est écrit hors du dossier donné.
- Un fichier par jour UTC : `mcp-proxy-AAAA-MM-JJ.log`. Le jour d'une ligne est celui de l'horloge injectée
  (`WallClock`) au début de l'enregistrement. Rétention : à l'ouverture et à chaque changement de jour, les fichiers
  `mcp-proxy-*.log` de plus de `log_retention_days` jours (aujourd'hui compris) sont supprimés ; aucun autre fichier
  du dossier n'est touché. Taille : 16 Mio par jour (`DEFAULT_MAX_FILE_BYTES`) ; au-delà, une ligne fixe signale que
  le journal du jour est plein, et plus rien n'est écrit avant le lendemain.
- File d'écriture bornée (4096 lignes) : une ligne qui n'y entre pas est perdue plutôt que d'attendre, et le nombre
  de lignes perdues est écrit dans le fichier à la ligne suivante.
- Le contenu est gardé et écrit pour tous les statuts **sauf** `ok`, `input_required` et `suspended`, à moins que
  l'appel soit lent (`trace::keeps_content`) : « en erreur » couvre `error`, `leak_masked`, `cancelled` ; « refusé »
  couvre `denied`, `refused`, `timeout`, `rate_limited`, `auth_required` ; « bloqué » est `blocked`.
- Ligne d'appel : `<horodatage UTC> INFO  mcp.call call_id=… status=… slow=… duration_ms=… client="…" env=… mcp=…
  tool="…" session=… policy=…` ; avec contenu, niveau `WARN` et en plus `explanation="…" masked_exact=…
  masked_generic=… request={…} response={…}`. Les textes sont des chaînes JSON (une valeur ne peut pas ouvrir une
  nouvelle ligne) ; requête et réponse sont tronquées à 64 Kio chacune (`…[tronqué]`).
- Lignes des bibliothèques : `trace::install_global(FileLog)` installe pour tout le processus un collecteur
  `tracing` qui écrit dans le même `FileLog`, et route les enregistrements du crate `log` (reqwest, native-tls) vers
  lui. Niveaux par défaut : `INFO` pour nos crates (`mcp_proxy*`), `WARN` pour les autres. Filtre indisponible : la
  ligne fixe de la spec 003 est écrite à la place de chaque enregistrement, lignes d'appel comprises.

### Journal pour l'interface

- `Journal::snapshot(filtre)` et `Journal::subscribe()` (diffusion des nouvelles entrées).
- `JournalEntry` : résumé pour toutes les lignes ; détails (trace, client, session, politique, requête et réponse
  filtrées) pour les lignes en erreur, lentes ou bloquées.
- Filtres côté cœur : texte, environnement, statut (`Tout`, `Problèmes`, `Politiques`).

Précisions de réalisation :

- `JournalEntry { seq, at, kind, status, slow, client, env, mcp, tool, duration, decision, summary, details }`,
  `kind` parmi `Call`, `ToolsChanged`, `Panic`. `details: Option<CallDetails { call_id, client, session, policy,
  request, response, filter_report }>`, présent selon la même règle que le contenu du fichier.
- `JournalFilter { text, env, status }`, critères cumulés. Texte : recherché sans casse ni accents
  (`trace::normalize`) dans le résumé, le client, l'outil, l'environnement, le MCP, la clé et le libellé du statut ;
  jamais dans le contenu. `Problèmes` : `error`, `leak_masked`, `blocked`, `timeout`, `rate_limited`,
  `auth_required`, `cancelled`, tout appel lent, et les panics. `Politiques` : `denied`, `refused`, `timeout`,
  `suspended`, `input_required`, et tout appel dont la politique demandait une confirmation (acceptée ou non).
  `timeout` est dans les deux.
- `CallStatus::label` donne le libellé français (`réussi`, `interdit`, `fuite masquée`…). `subscribe` rend un
  `broadcast::Receiver<JournalEntry>` : un abonné trop lent perd les plus anciennes (`Lagged`).
- Changements de liste d'outils (spec 008) : `gateway::journal::follow_tool_changes(événements du registre,
  journal)` fait de chaque `ToolsChanged` une entrée `ToolsChanged` et une ligne `mcp.tools` : « dev / kibana-logs :
  liste d'outils changée, 2 nouveaux (a, b) ; 1 modifié (c) ; 1 supprimé (d) » (20 noms au plus par catégorie).

### Export de diagnostic

- `diagnostics::export(dest)` produit `mcp-proxy-diagnostic-AAAAMMJJ-HHMM.zip` : fichiers de log (repassés par le
  filtre à l'export), version, système, résumé de la config (identifiants, libellés, hôtes), réglages non secrets,
  états des instances, rapport d'auto-diagnostic s'il existe (7f).
- Aucun secret, aucun contenu non filtré.

Précisions de réalisation :

- `diagnostics::Diagnostics { version, system, config, log_dir, filter, clock, log }` et
  `Diagnostics::export(dest, &DiagnosticState { settings, instances, doctor_report })`. `dest` est un dossier (nom
  daté en UTC) ou le chemin de l'archive choisi par l'utilisateur. Archive écrite à côté (`.part`) puis renommée :
  un échec ne laisse rien.
- Contenu : `LISEZMOI.txt`, `version.txt`, `systeme.txt`, `config.txt` (identifiants, libellés, hôtes sans chemin
  ni requête), `reglages.json` (forme du fichier de réglages), `instances.txt` (`gateway::journal::instance_lines`),
  `auto-diagnostic.txt` (le rapport donné, sinon le dernier `doctor-*.txt` du dossier des logs), `logs/mcp-proxy-*.log`
  (les 64 derniers Mio de chacun au plus).
- **Chaque fichier** de l'archive passe par le filtre au moment de l'export. Filtre indisponible : aucune archive.

### Panics

- Hook de panic : emplacement et message filtré dans le journal. Aucune donnée de requête.

Précisions de réalisation : `trace::install_panic_hook(journal)` remplace le hook par défaut, qui n'est pas
appelé (il écrirait le message non filtré sur la sortie d'erreur, lue par le client LLM en mode pont). Le hook
écrit `ERROR panic location="fichier:ligne:colonne" message="…"` et une entrée `Panic`, message borné à 1000
caractères, puis vide la file du fichier. Il ne prend pas le verrou du tampon s'il est déjà tenu.

## Tests

- **EX-009-01** — Le tampon respecte ses deux bornes et évince les plus anciens.
- **EX-009-02** — Un secret présent dans une requête ou une réponse n'apparaît ni dans le tampon, ni dans le fichier, ni dans
  l'export (recherche de toutes les variantes).
- **EX-009-03** — Appel normal : une ligne sans contenu. Appel en erreur, bloqué ou lent : contenu présent et filtré.
- **EX-009-04** — Seuil de lenteur configurable (horloge de test).
- **EX-009-05** — Rotation et rétention.
- **EX-009-06** — `subscribe` reçoit les nouvelles entrées ; les filtres texte, environnement et statut sélectionnent les bonnes
  lignes.
- **EX-009-07** — L'archive contient les fichiers attendus et aucun secret.
- **EX-009-08** — Un `tracing::error!` contenant un secret est masqué dans le fichier.
- **EX-009-09** — Chaque appel a un span `mcp.call` avec ses neuf champs ; `client` et `tool` sont filtrés, aucun
  champ ne contient de secret.
- **EX-009-10** — Un changement de liste d'outils (nouveaux, modifiés, supprimés) donne une entrée de journal et une
  ligne du fichier.
- **EX-009-11** — Panic : emplacement et message filtré dans le journal et le fichier, aucune donnée de requête.
- **EX-009-12** — Les enregistrements des bibliothèques (`tracing` et `log`) passent par le filtre ; filtre
  indisponible : la ligne fixe de la spec 003 à la place de chaque ligne.
- **EX-009-13** — L'export repasse les logs par le filtre : un secret connu au moment de l'export mais écrit avant
  d'être connu n'est pas dans l'archive ; filtre indisponible, aucune archive.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Onglet Journal (6c).

## Amendements

- **2026-11-22** — Réalisation (I009.1 à I009.15). Ajouts : précisions de réalisation dans chaque section ; critères
  `EX-009-09` à `EX-009-13`, pour ce que le texte demandait sans critère (span, changements de liste, panic, lignes
  des bibliothèques, refiltrage à l'export). Écarts : le contenu est gardé pour tous les statuts sauf `ok`,
  `input_required` et `suspended` (le texte ne citait que « erreur, bloqué, refusé ») ; les trois statuts ajoutés par
  la spec 007 sont rangés dans `Problèmes` (`cancelled`) ou `Politiques` (`suspended`, `input_required`) ; jours et
  heures en UTC (fichiers, archive) ; taille d'un fichier bornée par jour, sans clé de config ; une ligne qui ne
  tient pas dans la file d'écriture est perdue et comptée. Pour `EX-009-04`, l'« horloge de test » est la durée
  portée par le `CallRecord` : le journal ne lit aucune horloge pour le seuil. Raison : points laissés ouverts par le
  texte, tranchés par l'option la plus sûre ; exigence de la spec 007 qu'un `record` ne bloque jamais.
