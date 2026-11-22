# [Étape 5d] Réglages scellés (HMAC + DPAPI)

> Référence commune : `00-cadrage.md`. Dépend de : étapes 2c, 3a, 5a. Option retenue au QCM (D13).

## Objectif

Empêcher qu'un programme tiers assouplisse les réglages en modifiant un fichier : par exemple un agent LLM ayant
accès au disque, qui changerait le profil ou exposerait PROD.

## À faire

- Clé HMAC aléatoire de 32 octets, protégée par DPAPI (`CryptProtectData`, portée utilisateur) et stockée dans
  `%LOCALAPPDATA%\MCP Proxy\settings.key`.
- `settings.json` est accompagné d'un HMAC-SHA256 de son contenu canonique. Vérification à chaque chargement,
  comparaison en temps constant.
- Fichier modifié hors de l'appli, HMAC absent ou clé illisible : **retour aux réglages les plus stricts** — profil
  marqué `strictest` dans la config, aucun environnement exposé. L'utilisateur est prévenu (notification et
  bandeau) et doit choisir à nouveau son profil. L'événement est journalisé.
- Le démarrage automatique et les autres préférences sans effet sur la sécurité sont remis à leur valeur par
  défaut dans ce cas.
- Le même scellement protège les fichiers du **cache du catalogue** (3a). Un fichier de cache invalide est ignoré :
  retour à l'instantané embarqué.
- Logique dans `core::sealed`, indépendante de la plateforme : `SealedStore` paramétré par un trait
  `KeyProtector` — DPAPI sous Windows, implémentation de développement ailleurs.
- `SealedSettingsStore` implémente `SettingsStore` et remplace le fichier JSON simple dans l'appli.
- Limite à écrire dans `DECISIONS.md` : un programme qui tourne sous le compte de l'utilisateur peut lui aussi
  appeler DPAPI. Le scellement relève la barre, il ne la rend pas infranchissable.

## Tests

- Aller-retour ; contenu modifié d'un octet → réglages stricts ; HMAC absent ; clé absente ; clé d'un autre
  fichier.
- Cache de catalogue altéré → ignoré, instantané embarqué utilisé.
- Le repli ne donne jamais des réglages plus souples que ceux du profil `strictest`.
- Tests `#[cfg(windows)]` pour DPAPI, compilés en contrôle croisé.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- Modifier `settings.json` à la main (changer le profil) puis relancer : profil « Lecture seule », aucun
  environnement exposé, notification affichée.
- Un usage normal (changer de profil dans l'appli, relancer) conserve les réglages.
