---
id: "014"
titre: "Réglages scellés (HMAC + DPAPI)"
etape: "5d"
statut: "en cours"
depend_de: ["004", "005", "011"]
suivi: "github#15"
---

# 014 — Réglages scellés (HMAC + DPAPI)

> Option retenue au QCM (D13).

## Objectif

Empêcher qu'un programme tiers assouplisse les réglages en modifiant un fichier : par exemple un agent LLM ayant
accès au disque, qui changerait le profil ou exposerait PROD.

## À faire

- Clé HMAC aléatoire de 32 octets, protégée par DPAPI au niveau utilisateur par l'enrobage WinRT du core
  (`mcp_proxy_core::platform::windows::protect` / `unprotect`, spec 002), et stockée dans
  `%LOCALAPPDATA%\MCP Proxy\settings.key`. Plus de `CryptProtectData`, qui n'existe qu'en `unsafe` (D18).
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
  `KeyProtector` — l'enrobage DPAPI WinRT du core sous Windows, implémentation de développement ailleurs. Réutiliser
  `secrets::KeyWrapper` (spec 002) s'il convient tel quel.
- `SealedSettingsStore` implémente `SettingsStore` et remplace le fichier JSON simple dans l'appli.
- Limite à écrire dans `DECISIONS.md` : un programme qui tourne sous le compte de l'utilisateur peut lui aussi
  appeler DPAPI. Le scellement relève la barre, il ne la rend pas infranchissable.

## Tests

- **EX-014-01** — Aller-retour ; contenu modifié d'un octet → réglages stricts ; HMAC absent ; clé absente ; clé d'un autre
  fichier.
- **EX-014-02** — Cache de catalogue altéré → ignoré, instantané embarqué utilisé.
- **EX-014-03** — Le repli ne donne jamais des réglages plus souples que ceux du profil `strictest`.
- **EX-014-04** — Tests `#[cfg(windows)]` pour DPAPI, compilés en contrôle croisé.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-014-05** (manuel) — Modifier `settings.json` à la main (changer le profil) puis relancer : profil « Lecture seule », aucun
  environnement exposé, notification affichée.
- **EX-014-06** (manuel) — Un usage normal (changer de profil dans l'appli, relancer) conserve les réglages.
- **EX-014-07** (manuel) — `settings.key` ne contient pas la clé en clair ; copié avec `settings.json` dans le
  profil d'un autre utilisateur Windows, il ne s'ouvre pas : réglages stricts et notification.

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur) : la clé HMAC est protégée par l'enrobage DPAPI
  WinRT du core (spec 002), pas par `CryptProtectData`. `EX-014-04` reste vrai (tests DPAPI compilés en contrôle
  croisé). Aucun critère retiré ; ajouté : `EX-014-07`.
