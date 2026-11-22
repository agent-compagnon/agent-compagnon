---
id: "016"
titre: "ui — socle Vue/shadcn-vue, en-tête et profil, onglet Connexions"
etape: "6a"
statut: "en cours"
depend_de: ["011"]
suivi: "github#17"
---

# 016 — ui — socle Vue/shadcn-vue, en-tête et profil, onglet Connexions

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

### Commandes de la fenêtre (réalisation)

- Câblage mince dans `app/src/interface.rs`, logique dans `mcp_proxy_gateway::interface` (testée sous Linux) :
  `connect_session(session)` (ramène devant la fenêtre de connexion déjà ouverte, sinon lance la connexion et
  répond aussitôt), `disconnect_session(session)` (la carte passe à « expirée »), `set_profile(profile)`,
  `resume_gateway` (`GatewayControl::resume`), `show_confirmations` (« Voir » : la fenêtre de détail de chaque
  demande en attente, spec 013 ; aucune réponse n'est donnée depuis le bandeau).
- Chaque commande est déclarée dans `build.rs` et donnée par la seule capacité `main` (fenêtre `main`) ; elle
  revérifie que la fenêtre appelante est `main`. Les identifiants reçus sont vérifiés contre la config ; une erreur
  est un texte français sans secret, montré par la fenêtre.
- `set_profile` applique `UserSettings::choose_profile` (environnements exposés ramenés aux valeurs par défaut du
  profil, durcissements conservés), journalise « profil « X » choisi par l'utilisateur » et retire le bandeau
  « réglages réinitialisés » ; un échec d'enregistrement laisse le profil appliqué et ajoute une alerte
  `settingsNotSaved`.
- L'instantané gagne `sessions[].state.requestedBy` (client LLM qui attend la connexion, vide s'il est inconnu,
  `null` si seul l'utilisateur l'a demandée) et `profile.choices[].description` (phrase du profil, champ facultatif
  `description` des profils de la config).
- Le sous-menu provisoire « Profil » du menu de l'icône (spec 014) est retiré.

### Fenêtres annexes

- Page de saisie Basic (5b) et page de détail d'une confirmation (5c), dans le même bundle, sur des routes dédiées,
  avec le même thème.

## Tests

- **EX-016-01** — Vitest : store, cartes dans chaque état, bandeau présent ou absent, sélecteur de profil, dialogue de premier
  lancement.
- **EX-016-02** — Playwright sur `vite preview` avec le backend simulé : captures de l'onglet Connexions, en clair et en sombre,
  pour chaque scénario. Vérifier visuellement que l'orange n'apparaît que sur les éléments en attente.
- **EX-016-03** — Accessibilité de base : navigation au clavier, libellés, contrastes.
- **EX-016-06** — Instantané pour l'interface : le client LLM qui attend une connexion y figure (et seulement tant
  qu'il attend) ; l'instantané sérialisé de la config d'exemple, dans un état qui remplit chaque champ facultatif,
  est le fichier `ui/src/api/snapshot.example.json`, que les tests de `ui/` valident strictement contre
  `types.ts`.
- **EX-016-07** — Commandes de la fenêtre, sur l'assemblage réel : « Se connecter » puis « Déconnecter » (état
  « expirée », données des sites effacées) ; choix du profil (environnements par défaut du profil, durcissements
  conservés, bandeau de réinitialisation retiré, choix scellé) ; « Voir » ouvre les fenêtres de détail sans rien
  approuver ; « Reprendre » lève la suspension ; un identifiant inconnu est refusé.

## Vérification

`cargo xtask ci` vert (lint, typecheck, tests et build de `ui/` compris).

## Recette manuelle (Windows)

- **EX-016-04** (manuel) — Premier lancement : le choix du profil est demandé, puis l'onglet Connexions s'affiche.
- **EX-016-05** (manuel) — Les cartes reflètent l'état réel pendant une connexion et une déconnexion.

## Hors périmètre

Onglets MCP (6b) et Journal (6c).
