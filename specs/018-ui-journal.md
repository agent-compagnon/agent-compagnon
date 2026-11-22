---
id: "018"
titre: "ui — onglet Journal"
etape: "6c"
statut: "en cours"
depend_de: ["009", "016"]
suivi: "github#19"
---

# 018 — ui — onglet Journal

## Objectif

Suivre les appels en direct et comprendre un problème sans quitter l'appli, à partir de données déjà filtrées.

## À faire

- **Logs en direct** : les nouvelles entrées arrivent par événement (`journal-entry`) ; état initial par
  `get_journal`.
- **Recherche** texte, insensible aux accents et à la casse (même normalisation qu'en 6b).
- **Filtre par environnement**.
- **Filtre de statut** : `Tout`, `Problèmes` (erreur, bloqué, lent, fuite masquée, délai), `Politiques` (interdit,
  confirmation acceptée ou refusée).
- Bouton **En direct / En pause**. En pause, la liste ne bouge plus et un compteur indique les entrées arrivées
  entre-temps ; la reprise les affiche.
- Bouton **Exporter un diagnostic** : dialogue d'enregistrement, puis `export_diagnostic`. Message de fin avec le
  chemin de l'archive.
- Chaque ligne : heure, client, environnement, MCP, outil, statut, durée.
- Les lignes **en erreur, lentes ou bloquées se déplient** et montrent : identifiant de trace, client, session,
  politique et son explication, requête et réponse filtrées (JSON lisible). Les lignes normales ne se déplient pas :
  leur contenu n'est pas conservé pour l'affichage.
- Liste virtualisée : fluide avec plusieurs milliers d'entrées. La mémoire côté interface est bornée.
- L'interface n'ajoute aucun filtrage de secrets : elle n'affiche que ce que le cœur lui donne, déjà filtré.

### Vue du journal et commandes (réalisation)

- Vue en camelCase dans `mcp_proxy_gateway::journal_view` : `JournalView { entries }` (réponse de `get_journal`,
  du plus ancien au plus récent) et `EntryView` (`seq`, `at` en millisecondes, `kind` parmi `call`,
  `toolsChanged`, `panic`, `event`, `status` et `statusLabel` — libellé de `CallStatus::label` —, `slow`, `client`,
  `environment` et `mcp` en identifiants, `tool`, `durationMs`, `decision`, `summary`, `problem`, `policy`,
  `details`). `problem` et `policy` sont `is_problem` et `is_policy` du cœur : l'interface ne reclasse rien.
  `details` (`callId`, `client`, `session`, `policy` avec son explication, `request`, `response`, `masked` : nombre
  de valeurs masquées et leurs genres, jamais leur valeur) n'existe que si le cœur a gardé le contenu
  (`keeps_content`). Chaque envoi repasse par le filtre de sortie ; filtre indisponible : rien n'est envoyé.
- Contrat figé par `ui/src/api/journal.example.json`, écrit par `crates/gateway/tests/journal_contract.rs` et lu
  par les tests de `ui/` à travers les schémas stricts de `types.ts` (même mécanisme que l'instantané, I016.6).
- Événements : `journal-entry` (une entrée) et `journal-resync`. Un récepteur en retard (`Lagged`) ou une entrée
  qui ne passe pas le filtre donne `journal-resync` : la fenêtre redemande `get_journal` et fusionne par `seq`, sans
  doublon ni trou. `JournalFeed` (passerelle) décide ; l'appli ne fait qu'émettre.
- `export_diagnostic` : l'appli ouvre le dialogue d'enregistrement de Windows (`rfd`, fenêtre principale pour
  parent, filtre « Archive zip », nom `mcp-proxy-diagnostic-AAAAMMJJ-HHMM.zip`, dossier « Téléchargements » s'il
  existe), puis `Gateway::export_diagnostic(chemin)` écrit l'archive de la spec 009 (réglages, état des instances,
  logs refiltrés, système : version de Windows et de WebView2). La page ne donne jamais de chemin. Réponse :
  `{ status: "saved", path }` ou `{ status: "cancelled" }` (dialogue fermé) ; un échec est l'erreur de la commande,
  en français, sans contenu (« export de diagnostic impossible : … »), et n'écrit rien. Un nom sans `.zip` le
  reçoit. Le journal note « diagnostic exporté par l'utilisateur » (sans le chemin) ou l'échec.
- Les deux commandes sont déclarées dans `build.rs`, données par la seule capacité `main`, et revérifient la
  fenêtre appelante (I016.1).

## Tests

- **EX-018-01** — Arrivée d'entrées en direct ; pause puis reprise ; compteur d'entrées en attente.
- **EX-018-02** — Chaque filtre, seul et combiné ; recherche sur client, outil, MCP, statut.
- **EX-018-03** — Dépliage : présent sur une ligne en erreur, absent sur une ligne normale ; contenu attendu.
- **EX-018-04** — Export : commande appelée avec le chemin choisi ; message de succès et d'échec.
- **EX-018-05** — Borne mémoire : au-delà de la limite, les plus anciennes entrées sortent.
- **EX-018-06** — Playwright : captures (liste, ligne dépliée, en pause), en clair et en sombre.
- **EX-018-10** — Vue du journal : les entrées de chaque genre et de chaque famille de statut, sérialisées comme la
  fenêtre les reçoit, sont le fichier `ui/src/api/journal.example.json`, validé strictement par `types.ts` ;
  `problem` et `policy` sont ceux du cœur ; détails présents seulement pour un contenu gardé, sans valeur masquée.
- **EX-018-11** — Export sur l'assemblage réel : archive au chemin choisi (`.zip` ajouté au besoin), nom proposé
  daté, dossier « Téléchargements » ; un échec est un message clair, ne laisse aucun fichier et est journalisé.
- **EX-018-12** — Un abonné en retard reçoit une resynchronisation au lieu d'une erreur, puis la suite ; la
  fenêtre redemande alors le journal et le fusionne sans doublon ni perte.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-018-07** (manuel) — Un appel réussi, un appel interdit, un appel lent et un appel bloqué apparaissent avec le bon statut.
- **EX-018-08** (manuel) — La ligne bloquée se déplie ; aucun secret n'y est visible.
- **EX-018-09** (manuel) — L'archive exportée s'ouvre et ne contient aucun secret.
