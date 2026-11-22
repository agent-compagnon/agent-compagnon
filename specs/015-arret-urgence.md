---
id: "015"
titre: "Arrêt d'urgence dans le menu de l'icône"
etape: "5e"
statut: "implémentée"
depend_de: ["007", "010", "011", "012"]
suivi: "github#16"
---

# 015 — Arrêt d'urgence dans le menu de l'icône

> Option retenue au QCM (D13).

## Objectif

Donner à l'utilisateur deux gestes immédiats quand quelque chose lui paraît anormal.

## À faire

### « Suspendre la passerelle » / « Reprendre »

- Suspendue, la passerelle refuse **tous** les `tools/call` avec
  `MCP Proxy : passerelle suspendue par l'utilisateur.`, sans aucun appel amont, statut `suspended` au journal. Les
  appels en attente d'une connexion ou d'une confirmation sont refusés aussitôt.
  - Réalisation : la chaîne fait courir chaque attente (connexion, confirmation par notification, confirmation dans
    le chat en ancien protocole) contre la suspension, lue dans le `watch` des réglages ; l'abandon de la demande
    retire son toast et sa fenêtre (spec 013). L'étape amont revérifie la suspension avant d'appeler.
    `GatewayControl::suspend` refuse en plus toutes les confirmations encore en attente
    (`PendingConfirmations::refuse_all`, qui, à la différence de `shutdown`, n'empêche pas les demandes suivantes).
  - Une fenêtre de connexion ouverte reste ouverte : seuls les appels qui l'attendent sont refusés (D4 : rien ne se
    ferme dans le dos de l'utilisateur).
- La liste d'outils ne change pas : les clients ne voient pas leurs outils disparaître.
- L'état est persistant (`UserSettings::gateway_suspended`, dans le fichier scellé de la spec 014) : un redémarrage
  ne lève pas la suspension. Seul l'utilisateur la lève. Un échec d'enregistrement laisse la suspension appliquée
  jusqu'à l'arrêt et ajoute une alerte d'erreur `settingsNotSaved`. Un retour aux réglages les plus stricts
  (spec 014) remet les préférences à leur valeur par défaut, suspension comprise : aucun environnement n'est alors
  exposé (I015.6).
- L'icône prend un état visible : `AppStatus::Suspended` (icône grise marquée d'une pause), prioritaire sur
  « attention » et « erreur », infobulle « MCP Proxy — passerelle suspendue » suivie des autres éléments.
- Le bandeau de l'interface avec le bouton « Reprendre » est dessiné par la spec 016 à partir de
  `AppSnapshot::gateway_suspended` (commande `resume_gateway`, qui appelle `GatewayControl::resume`) ; l'élément du
  menu de l'icône devient aussi « Reprendre ».

### « Tout déconnecter »

- `SecretStore::wipe_all`, toutes les sessions repassent à « jamais connectée », effacement complet des données de
  navigation de la WebView (`clear_all_browsing_data`), fermeture des fenêtres de login et de la WebView gardienne.
  - Réalisation : `OnDemandAuthenticator::disconnect_all` annule chaque connexion en cours, efface tous les secrets
    (leurs variantes restent masquées pendant leur délai de grâce, spec 002), remet chaque session à « jamais
    connectée » (événement `Disconnected`), puis `AuthUi::clear_all_site_data` : l'appli ferme les fenêtres de
    connexion et de saisie Basic, efface le profil par la gardienne (`clear_all_browsing_data`, puis suppression de
    chaque cookie restant), puis détruit la gardienne ; la connexion suivante en crée une nouvelle. Les secrets sont
    effacés avant le profil : un échec de l'effacement du profil est journalisé et signalé par une boîte de message,
    sans rien changer aux secrets.
- Les appels en attente d'une connexion échouent avec un message clair : « connexion à « X » interrompue,
  l'utilisateur a déconnecté toutes les sessions ; appel refusé. Réessayez : une nouvelle connexion sera
  demandée. » (`SessionError::Disconnected`, statut `auth_required`).
- Entrée de journal. Pas de confirmation demandée : le geste doit être immédiat.

### Menu

« Ouvrir », « Suspendre la passerelle » ou « Reprendre », « Tout déconnecter », « Quitter ». (Le sous-menu
provisoire « Profil » de la spec 014 est retiré par la spec 016, qui choisit le profil dans la fenêtre.)

- La logique (`GatewayControl::suspend`, `resume`, `disconnect_all`) vit dans les crates ; l'appli ne fait que
  l'appeler. `GatewayControl` est un trait de `mcp_proxy_gateway::control`, implémenté par `Gateway`. Suspendre une
  passerelle déjà suspendue, ou reprendre une passerelle active, ne fait rien.
- Journal : entrées `event` « passerelle suspendue par l'utilisateur », « passerelle reprise par l'utilisateur »,
  « toutes les sessions déconnectées par l'utilisateur » (spec 014, I014.5).

## Tests

- **EX-015-01** — Suspendue : tout appel refusé, y compris un outil en `allow` ; aucun appel amont ; `CallRecord` au statut
  attendu.
- **EX-015-02** — Un appel en attente de confirmation ou de connexion est refusé au moment de la suspension.
- **EX-015-03** — Reprise : les appels repassent.
- **EX-015-04** — `disconnect_all` : plus aucun secret, toutes les sessions « jamais connectée », un appel suivant redemande une
  connexion.
- **EX-015-05** — L'état suspendu survit à un rechargement des réglages.
- **EX-015-09** — Suspendue, l'état de l'icône calculé par `AppStatus` est `suspended`, avant « attention » et « erreur », et
  l'infobulle le dit.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-015-06** (manuel) — « Suspendre » : un appel d'outil échoue avec le message ; l'icône change ; « Reprendre » rétablit.
- **EX-015-07** (manuel) — La suspension tient après redémarrage de l'appli.
- **EX-015-08** (manuel) — « Tout déconnecter » : toutes les cartes repassent à « jamais connectée » ; la connexion suivante redemande les
  identifiants.

## Amendements

- **2026-11-22** — Implémentation. Précisés : refus des appels en attente par la chaîne et `refuse_all` (I015.2),
  message de `SessionError::Disconnected` (I015.3), effacement du profil puis destruction de la gardienne
  (I015.4), état `Suspended` de l'icône (I015.5), effet d'un retour aux réglages stricts sur la suspension
  (I015.6), alerte en cas d'échec d'enregistrement. Le bandeau « Reprendre » de l'interface est reporté à la
  spec 016, qui dispose déjà de `gatewaySuspended` ; le menu de l'icône garde le sous-menu « Profil » de la
  spec 014. Ajouté : `EX-015-09` (état de l'icône). Aucun critère retiré.
- **2026-11-22** — Spec 016 : le menu de l'icône perd le sous-menu « Profil » (le profil se choisit dans la
  fenêtre) ; le bandeau « passerelle suspendue » de la fenêtre offre « Reprendre » (`resume_gateway`), et l'élément du
  menu suit l'état quelle que soit l'origine du changement. Comportement et critères de la 015 inchangés.
