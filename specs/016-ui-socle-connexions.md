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
- Réalisation : Tailwind CSS 4 (`@tailwindcss/vite`), composants shadcn-vue ajoutés comme sources dans
  `ui/src/components/ui/` (bouton, dialogue d'alerte, menu déroulant, onglets, boutons radio, libellé), icônes
  `@lucide/vue`, alias `@/` vers `ui/src/`. Jetons de couleur dans `ui/src/style.css` : base `neutral`,
  `--attention` et `--destructive` (avec leurs variantes texte et fond teinté) seules couleurs ; le mode sombre suit
  `prefers-color-scheme` (variante `dark:` liée au système, jamais à une classe). Aucun style ni script en ligne :
  la CSP de l'appli est servie aussi par `vite preview`, où les tests de bout en bout la voient.

### Couche d'accès au backend

- `ui/src/api/backend.ts` : interface. `tauri.ts` : `invoke` et `listen` (`@tauri-apps/api`). `mock.ts` : backend
  simulé avec des scénarios, choisis par `?scenario=` : `premier-lancement`, `sessions` (sessions dans chaque état),
  `connexion-demandee`, `confirmation`, `suspendue`, `reglages-reinitialises`, `profil-webview` (échec du vidage du
  profil WebView au démarrage), `mcp-mis-a-jour`. Ses données sont l'instantané de la config d'exemple écrit par la
  passerelle (`snapshot.example.json`), remanié par scénario ; il simule une connexion qui aboutit après un instant.
- Le backend simulé n'existe que dans la construction `--mode mock` (`npm run dev`, `npm run build:mock` vers
  `ui/dist-mock/`) : **l'interface tourne dans un simple navigateur**, ce qui permet de la développer, de la tester
  et de la capturer sous Linux. La construction de l'appli (`npm run build`, `ui/dist/`) ne le contient jamais :
  `scripts/check-dist.mjs` y cherche ses marques et fait échouer la construction. Sans Tauri et sans backend simulé,
  la page n'affiche qu'un message et n'a accès à aucune donnée.
- `types.ts` reflète les DTO Rust (`AppSnapshot` et ses parties) par des schémas stricts (`valibot`) dont les types
  sont déduits. Un instantané qui n'a pas exactement cette forme (clé en plus ou en moins, type faux) est refusé :
  la fenêtre dit « état illisible » au lieu d'en montrer une partie. Aucun secret n'y figure.
- Un store réactif tient le dernier `AppSnapshot` et s'abonne à `snapshot-changed` avant de demander
  `get_snapshot` ; une réponse plus ancienne qu'un événement reçu entre-temps est ignorée. Une commande refusée
  affiche sa raison dans un bandeau rouge, que l'utilisateur ferme.

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
- Réalisation :
  - en-tête : nom, version, sélecteur (menu déroulant à boutons radio, chaque profil avec ses environnements
    exposés, sensibles nommés) ; le bouton « Redémarrer pour mettre à jour » s'affiche quand `update` n'est pas
    `null`, inactif jusqu'à la spec 020 qui apporte la commande ;
  - premier lancement (`profile.firstLaunch`, donc aussi après une réinitialisation des réglages) : dialogue
    d'alerte que ni Échap ni un clic à côté ne ferment ; un profil à choisir (aucun présélectionné), sa phrase
    (`description` de la config) et ses environnements ; le profil le plus strict en dernier ; le message de
    réinitialisation y est repris quand il existe ;
  - changement de profil : « Passer au profil « X » ? », phrase du profil, environnements exposés par défaut,
    sensibles, « Vos durcissements sont conservés. », « Annuler » / « Changer de profil » ;
  - bandeaux : confirmation en attente en orange avec « Voir » (ouvre la fenêtre de détail, n'autorise rien) ;
    passerelle suspendue en noir et blanc avec « Reprendre » ; alertes de niveau erreur en rouge (réglages
    réinitialisés, échec du vidage du profil WebView, réglage non enregistré). Les alertes d'information et le
    seuil d'outils exposés sont laissés aux onglets MCP et Journal (specs 017, 018).

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
- Réalisation : le bandeau orange n'apparaît que si un **client LLM** attend (`requestedBy`, « Un client LLM » s'il
  n'a pas de nom) ; une connexion demandée depuis la carte n'a que son point orange. L'heure de fin est l'heure
  locale de `at` (« active jusqu'à demain 09:10 » le lendemain). Services : les MCP qui utilisent la session dans
  les environnements exposés, dans l'ordre de la config, suivis de leurs environnements pour une session globale
  (« GitLab Ops en DEV, REC · Jira en REC ») ; « Aucun service exposé » sinon. Sans environnement exposé, la
  section le dit. Les deux boutons de carte sont nommés pour les lecteurs d'écran (« Se connecter à SSO DEV »).

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
- Réalisation : ce sont des pages dédiées (`basic.html`, `confirm.html`, entrées de la même construction Vite),
  pas des routes d'une même page : chaque fenêtre ne charge que son code et garde sa seule commande. Elles prennent
  le thème et les jetons de `style.css`, et leurs textes passent dans `fr.ts` ; leur comportement (specs 012,
  013) est inchangé.

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

- **EX-016-08** — Le backend simulé n'est jamais dans la construction de l'appli : `npm run build` (donc
  `cargo xtask ci`) échoue si `ui/dist/` contient une de ses marques.

## Vérification

`cargo xtask ci` vert (lint, typecheck, tests et build de `ui/` compris).

## Recette manuelle (Windows)

- **EX-016-04** (manuel) — Premier lancement : le choix du profil est demandé, puis l'onglet Connexions s'affiche.
- **EX-016-05** (manuel) — Les cartes reflètent l'état réel pendant une connexion et une déconnexion.
- **EX-016-09** (manuel) — Bandeaux et commandes dans l'appli : un appel qui attend une connexion affiche le bandeau
  orange au nom du client ; une confirmation en attente affiche « Voir », qui ouvre sa fenêtre de détail sans rien
  autoriser ; « Reprendre » lève la suspension posée par le menu de l'icône ; changer de profil par l'en-tête
  demande confirmation puis expose les environnements du profil ; le menu de l'icône n'a plus de « Profil ».
- **EX-016-10** (manuel) — Thème : la fenêtre principale, la saisie Basic et la fenêtre de détail suivent le mode
  clair ou sombre de Windows, sans erreur de CSP ni texte illisible.

## Hors périmètre

Onglets MCP (6b) et Journal (6c).

## Amendements

- **2026-11-22** — Implémentation. Écarts au texte : le backend simulé n'est choisi qu'à la construction
  (`--mode mock`) et non à l'exécution en l'absence de Tauri, pour qu'il ne puisse pas se retrouver dans l'appli
  (I016.10) ; scénarios ajoutés (`suspendue`, `reglages-reinitialises`, `profil-webview`), le scénario « journal
  fourni » attend le DTO du journal (spec 018) ; les fenêtres annexes sont des pages dédiées et non des routes
  (I016.13) ; seules les alertes de niveau erreur sont des bandeaux globaux (I016.12) ; le bouton de mise à jour est
  inactif jusqu'à la spec 020. Ajouts : `description` des profils (I016.5), `requestedBy` de l'instantané (I016.3),
  commandes `resume_gateway` et `show_confirmations` (I016.4), retrait du menu « Profil » de l'icône (I016.7).
  Critères ajoutés : `EX-016-06` à `EX-016-08` (automatiques), `EX-016-09` et `EX-016-10` (recette). Aucun critère
  retiré.

