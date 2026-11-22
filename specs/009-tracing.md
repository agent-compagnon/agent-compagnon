---
id: "009"
titre: "Tracing — spans, tampon circulaire, journal fichier filtré, export de diagnostic"
etape: "3e"
statut: "en cours"
depend_de: ["003", "007"]
suivi: "github#10"
---

# 009 — Tracing — spans, tampon circulaire, journal fichier filtré, export de diagnostic

## Objectif

Pouvoir comprendre ce qui s'est passé pour chaque appel, sans jamais écrire un secret.

## À faire

### Span par appel

Span `mcp.call` avec : `call_id`, `client`, `env`, `mcp`, `tool`, `session`, `policy`, `status`, `duration_ms`.

### `CallRecord` et tampon circulaire

- `CallRecord { id, started_at, duration, client, env, mcp, tool, session, policy, status, request, response,
  filter_report, slow }`. `policy` porte la décision et son explication (2c).
- Statuts : `ok`, `error`, `denied` (politique), `refused` (utilisateur), `timeout` (confirmation), `blocked`
  (secret dans les arguments), `rate_limited`, `auth_required`, `leak_masked`.
- Tampon circulaire en mémoire, borné en nombre (`ring_capacity`) et en octets (`ring_max_bytes`).
- Le contenu (requête et réponse) est filtré **avant** d'entrer dans le tampon : il n'y a jamais de contenu brut,
  même en mémoire.
- Implémente le `TraceSink` de 3c.

### Journal fichier

- Dossier `%LOCALAPPDATA%\MCP Proxy\logs\` (chemin injecté). Rotation quotidienne, rétention `log_retention_days`,
  taille bornée.
- Pour un appel normal : une ligne de résumé, **sans contenu**.
- Le contenu complet d'un appel n'est écrit que s'il est en erreur, bloqué, refusé ou lent (durée ≥
  `slow_call_threshold_secs`, 5 s par défaut).
- Toutes les lignes passent par le `MakeWriter` filtrant de 2b, y compris celles des bibliothèques.

### Journal pour l'interface

- `Journal::snapshot(filtre)` et `Journal::subscribe()` (diffusion des nouvelles entrées).
- `JournalEntry` : résumé pour toutes les lignes ; détails (trace, client, session, politique, requête et réponse
  filtrées) pour les lignes en erreur, lentes ou bloquées.
- Filtres côté cœur : texte, environnement, statut (`Tout`, `Problèmes`, `Politiques`).

### Export de diagnostic

- `diagnostics::export(dest)` produit `mcp-proxy-diagnostic-AAAAMMJJ-HHMM.zip` : fichiers de log (repassés par le
  filtre à l'export), version, système, résumé de la config (identifiants, libellés, hôtes), réglages non secrets,
  états des instances, rapport d'auto-diagnostic s'il existe (7f).
- Aucun secret, aucun contenu non filtré.

### Panics

- Hook de panic : emplacement et message filtré dans le journal. Aucune donnée de requête.

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

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Onglet Journal (6c).
