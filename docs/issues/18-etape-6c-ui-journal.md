# [Étape 6c] ui — onglet Journal

> Référence commune : `00-cadrage.md`. Dépend de : étapes 3e et 6a.

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

## Tests

- Arrivée d'entrées en direct ; pause puis reprise ; compteur d'entrées en attente.
- Chaque filtre, seul et combiné ; recherche sur client, outil, MCP, statut.
- Dépliage : présent sur une ligne en erreur, absent sur une ligne normale ; contenu attendu.
- Export : commande appelée avec le chemin choisi ; message de succès et d'échec.
- Borne mémoire : au-delà de la limite, les plus anciennes entrées sortent.
- Playwright : captures (liste, ligne dépliée, en pause), en clair et en sombre.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- Un appel réussi, un appel interdit, un appel lent et un appel bloqué apparaissent avec le bon statut.
- La ligne bloquée se déplie ; aucun secret n'y est visible.
- L'archive exportée s'ouvre et ne contient aucun secret.
