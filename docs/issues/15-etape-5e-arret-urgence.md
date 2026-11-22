# [Étape 5e] Arrêt d'urgence dans le menu de l'icône

> Référence commune : `00-cadrage.md`. Dépend de : étapes 3c, 4, 5a, 5b. Option retenue au QCM (D13).

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

- Suspendue : tout appel refusé, y compris un outil en `allow` ; aucun appel amont ; `CallRecord` au statut
  attendu.
- Un appel en attente de confirmation ou de connexion est refusé au moment de la suspension.
- Reprise : les appels repassent.
- `disconnect_all` : plus aucun secret, toutes les sessions « jamais connectée », un appel suivant redemande une
  connexion.
- L'état suspendu survit à un rechargement des réglages.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- « Suspendre » : un appel d'outil échoue avec le message ; l'icône change ; « Reprendre » rétablit.
- La suspension tient après redémarrage de l'appli.
- « Tout déconnecter » : toutes les cartes repassent à « jamais connectée » ; la connexion suivante redemande les
  identifiants.
