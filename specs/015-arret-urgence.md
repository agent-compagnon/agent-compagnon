---
id: "015"
titre: "Arrêt d'urgence dans le menu de l'icône"
etape: "5e"
statut: "en cours"
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
  `MCP Proxy : passerelle suspendue par l'utilisateur.`. Les appels en attente d'une connexion ou d'une confirmation
  sont refusés aussitôt.
- La liste d'outils ne change pas : les clients ne voient pas leurs outils disparaître.
- L'état est persistant (`UserSettings::gateway_suspended`) : un redémarrage ne lève pas la suspension. Seul
  l'utilisateur la lève.
- L'icône prend un état visible, et un bandeau s'affiche dans l'interface avec le bouton « Reprendre ».

### « Tout déconnecter »

- `SecretStore::wipe_all`, toutes les sessions repassent à « jamais connectée », effacement complet des données de
  navigation de la WebView (`clear_all_browsing_data`), fermeture des fenêtres de login et de la WebView gardienne.
- Les appels en attente d'une connexion échouent avec un message clair.
- Entrée de journal. Pas de confirmation demandée : le geste doit être immédiat.

### Menu

« Ouvrir », « Suspendre la passerelle » ou « Reprendre », « Tout déconnecter », « Quitter ».

- La logique (`GatewayControl::suspend`, `resume`, `disconnect_all`) vit dans les crates ; l'appli ne fait que
  l'appeler.

## Tests

- **EX-015-01** — Suspendue : tout appel refusé, y compris un outil en `allow` ; aucun appel amont ; `CallRecord` au statut
  attendu.
- **EX-015-02** — Un appel en attente de confirmation ou de connexion est refusé au moment de la suspension.
- **EX-015-03** — Reprise : les appels repassent.
- **EX-015-04** — `disconnect_all` : plus aucun secret, toutes les sessions « jamais connectée », un appel suivant redemande une
  connexion.
- **EX-015-05** — L'état suspendu survit à un rechargement des réglages.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-015-06** (manuel) — « Suspendre » : un appel d'outil échoue avec le message ; l'icône change ; « Reprendre » rétablit.
- **EX-015-07** (manuel) — La suspension tient après redémarrage de l'appli.
- **EX-015-08** (manuel) — « Tout déconnecter » : toutes les cartes repassent à « jamais connectée » ; la connexion suivante redemande les
  identifiants.
