# Specs MCP Proxy

Ce dossier est **la mémoire du projet**. Il vit dans le dépôt git : il suit le code, ses branches et son historique,
et ne dépend d'aucune plateforme. Les tickets du gestionnaire d'issues ne sont que des pointeurs vers ces fichiers ;
on peut les recréer ailleurs à partir d'ici.

## Ce qu'on y trouve

| Fichier | Rôle | Change quand |
|---|---|---|
| `constitution.md` | Invariants de sécurité et règles de travail | Rarement, par amendement daté |
| `architecture.md` | Contraintes, stack, architecture, faits techniques vérifiés | Quand l'architecture change |
| `NNN-nom.md` | Une spec par fonctionnalité : objectif, comportement attendu, critères d'acceptation | Avec le code |
| `../DECISIONS.md` | Journal des décisions, en ajout seul | À chaque décision |
| `../docs/recette.md` | Vérifications manuelles sous Windows | Avec les specs |

## Règles pour rester aligné

1. **La spec et le code changent dans le même commit.** Un changement de comportement sans mise à jour de la spec
   n'est pas terminé.
2. **Chaque critère d'acceptation a un identifiant stable** : `EX-<spec>-<n>`, par exemple `EX-003-07`. Un
   identifiant n'est jamais réutilisé ni renuméroté. Un critère retiré garde sa ligne, barrée, avec la date.
3. **Chaque critère est prouvé.** Soit par un test qui cite son identifiant en commentaire
   (`// Exigence : EX-003-07`), soit, s'il est marqué `(manuel)`, par une ligne de `docs/recette.md` qui le cite.
4. **`cargo xtask trace` vérifie l'alignement** et fait partie de `cargo xtask ci` :
   - spec `implémentée` : tous ses critères sont cités, dans un test ou dans la recette ;
   - aucun test ni ligne de recette ne cite un identifiant inconnu ;
   - l'index ci-dessous correspond aux en-têtes des fichiers.
5. **Les changements se tracent**, sans dépendre d'un outil extérieur :
   - *pourquoi une spec a changé* : section « Amendements » en fin de spec, datée, avec la raison ;
   - *quel code a servi une spec* : ligne `Spec: NNN` en fin de message de commit, donc
     `git log --grep "Spec: 003"` ;
   - *pourquoi un choix a été fait* : `DECISIONS.md`. Une décision n'est jamais réécrite : elle est marquée
     « remplacée par … ».

## Statuts

`prévue` → `en cours` → `implémentée`. Hors parcours : `proposée` (pas encore validée), `backlog` (prévu, non
planifié), `à la demande`.

Les numéros sont des identifiants, pas un ordre : l'ordre de réalisation suit la colonne « Dépend de ».

## Index

<!-- index:début — généré par `cargo xtask trace --write-index`, ne pas modifier à la main -->
| Spec | Étape | Titre | Statut | Dépend de | Critères |
|---|---|---|---|---|---|
| [001](001-workspace-config.md) | 1 | Workspace Cargo, xtask ci, CI GitLab minimale, config TOML embarquée et validée | implémentée | — | 4 |
| [002](002-secrets.md) | 2a | core — store de secrets en mémoire | implémentée | 001 | 10 |
| [003](003-filtre.md) | 2b | core — filtre de sortie | prévue | 002 | 11 |
| [004](004-politiques.md) | 2c | core — moteur de politiques, profils et durcissement | prévue | 001 | 8 |
| [005](005-registre-catalogue.md) | 3a | gateway — registre d'outils, préfixage, exposition, catalogue embarqué + cache | prévue | 001, 004 | 6 |
| [006](006-transports-testkit.md) | 3b | gateway — transports MCP (côté clients, côté serveurs) et testkit | prévue | 002, 003, 005 | 7 |
| [007](007-middlewares.md) | 3c | gateway — chaîne de middlewares | prévue | 003, 004, 006 | 11 |
| [008](008-mises-a-jour-serveurs.md) | 3d | gateway — mises à jour des serveurs MCP | prévue | 005, 006, 007 | 10 |
| [009](009-tracing.md) | 3e | Tracing — spans, tampon circulaire, journal fichier filtré, export de diagnostic | prévue | 003, 007 | 8 |
| [010](010-auth.md) | 4 | auth — méthodes d'authentification et authentification à la demande | prévue | 002, 006, 007 | 11 |
| [011](011-app-squelette.md) | 5a | app — squelette Tauri, pont stdio (named pipe), instance unique, tray, réglages | prévue | 001 à 010 | 8 |
| [012](012-fenetres-login.md) | 5b | app — fenêtres de login WebView2 (JWT, cookie, Basic), profil InPrivate | prévue | 010, 011 | 7 |
| [013](013-notifications-confirmations.md) | 5c | app — notifications Windows et confirmations | prévue | 007, 011, 012 | 9 |
| [014](014-reglages-scelles.md) | 5d | Réglages scellés (HMAC + DPAPI) | prévue | 004, 005, 011 | 6 |
| [015](015-arret-urgence.md) | 5e | Arrêt d'urgence dans le menu de l'icône | prévue | 007, 010, 011, 012 | 8 |
| [016](016-ui-socle-connexions.md) | 6a | ui — socle Vue/shadcn-vue, en-tête et profil, onglet Connexions | prévue | 011 | 5 |
| [017](017-ui-mcp.md) | 6b | ui — onglet MCP | prévue | 008, 016 | 9 |
| [018](018-ui-journal.md) | 6c | ui — onglet Journal | prévue | 009, 016 | 9 |
| [019](019-installation.md) | 7a | Installation au premier lancement | prévue | 011 | 8 |
| [020](020-updater.md) | 7b | Updater Artifactory : signature ed25519, self-replace, canal bêta | prévue | 011, 019 | 11 |
| [021](021-pipeline-gitlab.md) | 7c | Pipeline GitLab de release | prévue | 001, 020 | 6 |
| [022](022-desinstallation.md) | 7d | Désinstallation propre | prévue | 019 | 5 |
| [023](023-config-clients.md) | 7e | Configuration automatique des clients LLM | prévue | 016, 019 | 8 |
| [024](024-doctor.md) | 7f | Auto-diagnostic (`doctor`) | prévue | 009, 011, 019, 020 | 7 |
| [025](025-module-jira-local.md) | 8 | Module MCP local d'exemple (`jira-local`) | prévue | 002, 007, 010 | 8 |
| [026](026-pont-resilient.md) | — | Pont stdio résilient : survivre au redémarrage de l'appli | proposée | 011, 020 | 8 |
| [027](027-config-signee.md) | — | Config signée téléchargée depuis Artifactory | backlog | — | 0 |
| [028](028-authenticode.md) | — | Signature Authenticode dans la CI | backlog | — | 0 |
| [029](029-exe-windows-a-la-demande.md) | — | Exe Windows cross-compilé depuis Linux | à la demande | — | 0 |
| [030](030-tracabilite.md) | — | Traçabilité des specs : identifiants de critères et `cargo xtask trace` | implémentée | 001 | 10 |
<!-- index:fin -->
