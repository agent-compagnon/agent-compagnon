---
id: "018"
titre: "ui — onglet Journal"
etape: "6c"
statut: "implémentée"
depend_de: ["009", "016"]
suivi: "github#19"
---

# 018 — ui — onglet Journal

## Objectif

Suivre les appels en direct et comprendre un problème sans quitter l'appli, à partir de données déjà filtrées.

## À faire

- **Logs en direct** : les nouvelles entrées arrivent par événement (`journal-entry`) ; état initial par
  `get_journal` ; après une perte (`journal-resync`), `get_journal` de nouveau.
- **Recherche** texte, insensible aux accents et à la casse (même normalisation qu'en 6b).
- **Filtre par environnement**.
- **Filtre de statut** : `Tout`, `Problèmes` (erreur, bloqué, lent, fuite masquée, délai), `Politiques` (interdit,
  confirmation acceptée ou refusée).
- Bouton **En direct / En pause**. En pause, la liste ne bouge plus et un compteur indique les entrées arrivées
  entre-temps ; la reprise les affiche.
- Bouton **Exporter un diagnostic** : `export_diagnostic` ouvre le dialogue d'enregistrement de l'appli, puis
  exporte. Message de fin avec le chemin de l'archive.
- Chaque ligne : heure, client, environnement, MCP, outil, statut, durée.
- Les lignes **dont le cœur a gardé le contenu se déplient** — en erreur, lentes, bloquées, et aussi refusées,
  interdites, délai dépassé… (`keeps_content`, spec 009 ; amendement du 2026-11-22) — et montrent : identifiant de
  trace, client, session, politique et son explication, requête et réponse filtrées (JSON lisible). Les lignes
  normales ne se déplient pas : leur contenu n'est pas conservé pour l'affichage.
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

### Onglet (réalisation)

- Le journal est suivi dès l'ouverture de la fenêtre, quel que soit l'onglet affiché (`ui/src/store/journal.ts`) :
  abonnement aux événements, puis `get_journal`, fusion par `seq`. Les lignes vont de la plus récente à la plus
  ancienne : en direct, une nouvelle entrée arrive en tête ; les plus anciennes sortent par le bas.
- Barre d'outils : recherche, filtre d'environnement (menu déroulant à boutons radio, « Tous » puis chaque
  environnement avec son nombre d'entrées, quels que soient les autres filtres), filtre de statut (« Tout »,
  « Problèmes », « Politiques », chacun avec son contenu en infobulle) ; puis « En direct » / « En pause », le
  nombre d'entrées affichées, en pause « 3 nouvelles entrées depuis la pause. » avec « Afficher », et « Exporter un
  diagnostic ».
- Recherche : `normalize` et `terms` de l'onglet MCP ; chaque mot doit se trouver dans un champ de la ligne :
  client, outil, libellé et identifiant de l'environnement et du MCP, statut (libellé du cœur et clé), « lent »,
  résumé (seul texte d'un événement). Les mots sont surlignés (`HighlightedText`, déplacé dans
  `components/common/`). Jamais dans la requête ni la réponse, comme le filtre du cœur.
- Colonnes : heure locale avec les secondes (précédée du jour si ce n'est pas aujourd'hui), client, environnement,
  MCP, outil, statut, durée (`182 ms`, `2,3 s`, `1 min 05 s`). Un événement, un changement de liste d'outils ou une
  panic occupe les colonnes client à outil avec son résumé, et son genre dans la colonne statut. Une marque « lent »
  suit le statut d'un appel lent.
- Dépliage : chevron et clic sur la ligne pour les seules entrées qui ont des `details` ; aucune autre ligne n'a de
  chevron ni ne réagit au clic. Détail : trace, client, session, politique (explication du moteur), ce que le filtre
  a masqué (nombre et genres), requête et réponse en JSON indenté, rendus comme texte.
- Couleurs : noir et blanc ; rouge pour `error`, `leakMasked` et les panics ; aucun orange (rien, dans un
  historique, n'attend l'utilisateur : la confirmation et la connexion en attente sont dans les bandeaux globaux).
  Un refus de politique, un appel bloqué ou une limite de débit restent noir et blanc.
- Liste virtualisée (`@tanstack/vue-virtual`), qui occupe la hauteur restante de la fenêtre et défile seule ;
  chaque ligne rendue est mesurée et suivie quand elle se déplie.
- Borne : `JOURNAL_LIMIT` = 5 000 entrées, affichées et en attente ensemble ; au-delà, les plus anciennes sortent,
  en pause aussi, et une resynchronisation ne les ramène pas. Une ligne le dit quand la borne est atteinte.
- Export : bouton inactif pendant l'export ; succès : bandeau noir et blanc « Diagnostic exporté : » et le chemin ;
  échec : bandeau rouge (« Export de diagnostic impossible : accès refusé. ») ; dialogue fermé : rien de nouveau,
  le message précédent reste.
- Alertes : les alertes d'information autres que le cache du catalogue (exclusion des rapports d'erreur Windows)
  sont des bandeaux noir et blanc en tête de l'onglet ; l'onglet MCP ne montre plus que celles du cache.
- Backend simulé : chaque scénario a un journal (les entrées du fichier de contrat et 50 appels générés) ;
  `journal-direct` ajoute un appel toutes les 1,2 s, `journal-volumineux` en a 5 000, `journal-vide` aucun ;
  `export-echec` et `export-annule` font échouer ou annuler l'export simulé. Les commandes simulées journalisent
  leurs événements comme la passerelle.

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
- **EX-018-13** (manuel) — L'onglet dans la fenêtre de l'appli, en clair et en sombre : entrées en direct, pause
  et reprise, infobulles et menu sans erreur de CSP ; « Exporter un diagnostic » ouvre le dialogue de Windows sur
  « Téléchargements », annuler n'affiche rien, enregistrer affiche le chemin ; un emplacement refusé donne un
  message rouge clair.

## Amendements

- **2026-11-22** — Implémentation (I018.1 à I018.11). Ajouts : précisions de réalisation (vue du journal et
  commandes, onglet) ; événement `journal-resync` pour une fenêtre en retard ; `get_journal` rend
  `{ entries }` ; `export_diagnostic` ouvre lui-même le dialogue d'enregistrement et rend `saved` avec le chemin ou
  `cancelled` ; la passerelle reçoit la description du système pour l'archive ; erreurs d'écriture de l'export en
  français (spec 009, `DiagnosticError::Io`) ; scénarios simulés `journal-direct`, `journal-volumineux`,
  `journal-vide`, `export-echec`, `export-annule`. Écarts au texte : les lignes en erreur, lentes ou bloquées ne
  sont pas les seules à se déplier : toute ligne dont le cœur a gardé le contenu se déplie (refusée, interdite,
  délai dépassé…, règle `keeps_content` de la spec 009), ce que demande aussi la recette `EX-018-07` ; l'alerte
  d'information de l'exclusion des rapports d'erreur Windows passe de l'onglet MCP à l'onglet Journal (I016.12).
  Critères ajoutés : `EX-018-10` à `EX-018-12` (automatiques), `EX-018-13` (recette). Aucun critère retiré.
