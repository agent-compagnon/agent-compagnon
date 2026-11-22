# [Étape 6a] ui — socle Vue/shadcn-vue, en-tête et profil, onglet Connexions

> Référence commune : `00-cadrage.md`. Dépend de : étape 5a (contrat `AppSnapshot`).

## Objectif

Une interface presque invisible : une fenêtre à 3 onglets, noir et blanc, où l'orange signale uniquement ce qui
attend une action. L'onglet Connexions est l'écran par défaut, le seul utile à 99 % des utilisateurs.

## À faire

### Socle

- Vue 3, Vite, TypeScript strict, Tailwind CSS, **shadcn-vue** (Reka UI dessous), icônes lucide, npm.
- Thème noir et blanc par défaut (base `neutral`), clair ou sombre selon Windows. Une variable `--attention`
  (orange) réservée à ce qui attend une action ; le rouge réservé aux erreurs.
- Textes en français, centralisés dans `ui/src/i18n/fr.ts`.
- Scripts : `lint` (eslint), `typecheck` (vue-tsc), `test` (Vitest + Vue Test Utils), `build`, `e2e` (Playwright,
  hors `cargo xtask ci`).

### Couche d'accès au backend

- `ui/src/api/backend.ts` : interface. `tauri.ts` : `invoke` et `listen`. `mock.ts` : backend simulé avec des
  scénarios (premier lancement, sessions variées, connexion demandée, confirmation en attente, MCP mis à jour,
  journal fourni).
- Sans Tauri, le backend simulé est utilisé : **l'interface tourne dans un simple navigateur**, ce qui permet de la
  développer, de la tester et de la capturer sous Linux.
- `types.ts` reflète les DTO Rust (`AppSnapshot` et ses parties). Aucun secret n'y figure.
- Un store réactif tient le dernier `AppSnapshot` et s'abonne à `snapshot-changed`.

### Cadre

- En-tête : nom de l'appli, **sélecteur de profil**, version, bouton « Redémarrer pour mettre à jour » quand une
  mise à jour est prête.
- 3 onglets : **Connexions** (par défaut), **MCP**, **Journal**. Les deux derniers restent des coquilles ici.
- Premier lancement : dialogue bloquant de choix du profil, avec une phrase par profil (Développeur, Support,
  Lecture seule).
- Changement de profil : dialogue de confirmation qui explique l'effet. Les environnements exposés reviennent aux
  valeurs par défaut du nouveau profil ; les durcissements de l'utilisateur sont conservés.
- Bandeaux globaux : passerelle suspendue (avec « Reprendre »), réglages réinitialisés, confirmation en attente
  (avec « Voir »).

### Onglet Connexions

- **Bandeau orange** si un LLM attend une connexion : « Claude Desktop attend une connexion à SSO PROD », avec le
  bouton « Se connecter ».
- Section **SSO globaux** : 2 cartes.
- Section **SSO par environnement** : une carte par environnement **activé**.
- Chaque carte : point d'état, libellé, état en toutes lettres (« jamais connectée », « active jusqu'à 17:42 »,
  « expirée », « connexion demandée »), services couverts (MCP qui utilisent la session), bouton « Se connecter »
  ou « Déconnecter ».
- Point d'état : plein pour active, contour pour jamais connectée, atténué pour expirée, orange pour connexion
  demandée.
- Commandes : `connect_session`, `disconnect_session`, `set_profile`.

### Fenêtres annexes

- Page de saisie Basic (5b) et page de détail d'une confirmation (5c), dans le même bundle, sur des routes dédiées,
  avec le même thème.

## Tests

- Vitest : store, cartes dans chaque état, bandeau présent ou absent, sélecteur de profil, dialogue de premier
  lancement.
- Playwright sur `vite preview` avec le backend simulé : captures de l'onglet Connexions, en clair et en sombre,
  pour chaque scénario. Vérifier visuellement que l'orange n'apparaît que sur les éléments en attente.
- Accessibilité de base : navigation au clavier, libellés, contrastes.

## Vérification

`cargo xtask ci` vert (lint, typecheck, tests et build de `ui/` compris).

## Recette manuelle (Windows)

- Premier lancement : le choix du profil est demandé, puis l'onglet Connexions s'affiche.
- Les cartes reflètent l'état réel pendant une connexion et une déconnexion.

## Hors périmètre

Onglets MCP (6b) et Journal (6c).
