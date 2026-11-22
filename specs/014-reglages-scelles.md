---
id: "014"
titre: "Réglages scellés (HMAC + DPAPI)"
etape: "5d"
statut: "implémentée"
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
  - Forme du fichier : une ligne d'en-tête `MCP-PROXY-SEAL v1 hmac-sha256=<64 hexadécimaux>`, puis le JSON des
    réglages tel que l'appli l'écrit. Le « contenu canonique » est ce contenu à l'octet près : le HMAC couvre une
    étiquette de domaine, l'usage du fichier (`settings.json`) et les octets exacts du contenu. Tout octet changé,
    espaces compris, invalide le sceau (I014.2).
  - Clé : `settings.key` contient un en-tête fixe puis la clé enveloppée par DPAPI ; en mémoire, elle reste
    enveloppée et n'est ouverte que le temps d'un scellement ou d'une vérification.
- Fichier modifié hors de l'appli, HMAC absent ou clé illisible : **retour aux réglages les plus stricts** — profil
  marqué `strictest` dans la config, aucun environnement exposé. L'utilisateur est prévenu (notification et
  bandeau) et doit choisir à nouveau son profil. L'événement est journalisé.
  - Clé illisible : absente, endommagée, d'un autre utilisateur Windows, ou ouverte par DPAPI mais d'une autre
    longueur. Clé d'un autre fichier (une autre installation) : le sceau ne correspond pas. Tous ces cas donnent les
    réglages par défaut (`UserSettings::default()`), dont les durcissements sont perdus (I004.11).
  - Bandeau : alerte de niveau erreur `settingsReset` (« Vos réglages n'ont pas pu être vérifiés (…) : les réglages
    les plus stricts les remplacent, aucun environnement n'est exposé. Choisissez de nouveau votre profil. ») ;
    notification « Réglages réinitialisés » (spec 013) ; entrée de journal de type `event` et ligne `mcp.event` dans
    le fichier du jour.
  - Le scellement suivant (premier enregistrement des réglages) remplace une clé absente ou illisible par une
    nouvelle clé : les fichiers scellés par l'ancienne ne s'ouvrent plus, ce qui ne mène qu'à l'état sûr. Un
    chargement ne crée jamais de clé (I014.3).
- **Première exécution après cette étape** : un `settings.json` écrit par une version sans scellement n'a pas de
  sceau. Il n'est **pas** migré : il est traité comme un fichier modifié hors de l'appli (réglages les plus stricts,
  bandeau, notification, journal) et l'utilisateur choisit de nouveau son profil. Raison : rien ne distingue un
  ancien fichier d'un fichier écrit par un tiers (I014.4). Les fichiers du cache du catalogue sans sceau sont
  ignorés de même (alerte d'information par instance) jusqu'à la liste réelle suivante.
- Le démarrage automatique et les autres préférences sans effet sur la sécurité sont remis à leur valeur par
  défaut dans ce cas.
- Le même scellement protège les fichiers du **cache du catalogue** (3a). Un fichier de cache invalide est ignoré :
  retour à l'instantané embarqué. L'usage scellé est `catalog/<mcp>.<env>` : un fichier recopié sous le nom d'une
  autre instance ne s'ouvre pas.
- Logique dans `core::sealed`, indépendante de la plateforme : `Sealer` (scellement et vérification avec une clé),
  paramétré par le trait `secrets::KeyWrapper` de la spec 002, réutilisé tel quel comme protecteur de clé —
  l'enrobage DPAPI WinRT du core sous Windows (`DpapiKeyWrapper`), implémentation de développement ailleurs.
- `SealedSettingsStore` implémente `SettingsStore` et remplace le fichier JSON simple dans l'appli ;
  `SealedCatalog` implémente `CacheSeal` (spec 005) avec le même `Sealer`. `Gateway::start` les reçoit par
  `RuntimeDeps::persistence` (`Persistence::sealed(&dirs, wrapper)` dans l'appli et les tests de l'assemblage).
- Choix du profil : par le sélecteur de profil de la fenêtre (spec 016), dont le dialogue de confirmation nomme les
  environnements exposés par le profil, sensibles compris ; seule la confirmation change les réglages, qui sont
  alors scellés. Après un retour aux réglages stricts, la fenêtre redemande le profil comme au premier lancement.
  Le sous-menu provisoire « Profil » du menu de l'icône (I014.6), qui tenait ce rôle tant que la spec 016
  n'existait pas, est retiré (amendement du 2026-11-22).
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
- ~~**EX-014-08** (manuel) — Mise à niveau : un `settings.json` écrit par une version sans scellement (ou à la main)
  donne au premier lancement les réglages stricts, le bandeau, la notification « Réglages réinitialisés » et une
  ligne `mcp.event` dans le journal ; un profil choisi par le menu « Profil » de l'icône tient au redémarrage.~~
  (retiré le 2026-11-22 : le menu « Profil » de l'icône n'existe plus, voir `EX-014-09`)
- **EX-014-09** (manuel) — Mise à niveau : un `settings.json` écrit par une version sans scellement (ou à la main)
  donne au premier lancement les réglages stricts, le bandeau, la notification « Réglages réinitialisés » et une
  ligne `mcp.event` dans le journal ; un profil choisi dans la fenêtre (spec 016) tient au redémarrage.

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur) : la clé HMAC est protégée par l'enrobage DPAPI
  WinRT du core (spec 002), pas par `CryptProtectData`. `EX-014-04` reste vrai (tests DPAPI compilés en contrôle
  croisé). Aucun critère retiré ; ajouté : `EX-014-07`.
- **2026-11-22** — Implémentation. Le trait `KeyProtector` n'est pas créé : `secrets::KeyWrapper` convient tel quel
  (I014.1) ; `SealedStore` s'appelle `Sealer`. Précisés : forme du fichier scellé et sens de « contenu canonique »
  (octets exacts, I014.2) ; remplacement d'une clé inutilisable au premier enregistrement (I014.3) ; première
  exécution après mise à niveau, sans migration (I014.4) ; injection par `RuntimeDeps::persistence`. Ajoutés :
  l'entrée de journal `event` (I014.5), le sous-menu provisoire « Profil » de l'icône, faute duquel aucun profil ne
  pourrait être rétabli avant la spec 016 (I014.6), et le critère `EX-014-08`. Aucun critère retiré.
- **2026-11-22** — Spec 016 : le profil se choisit dans la fenêtre (dialogue du premier lancement, sélecteur de
  l'en-tête et son dialogue de confirmation). Le sous-menu provisoire « Profil » du menu de l'icône est retiré, avec
  la question Oui/Non qui l'accompagnait (`view::profile_change_question`, `dialog::confirm`) ; le dialogue de la
  fenêtre en reprend le contenu (environnements exposés, sensibles nommés, durcissements conservés). Choisir un
  profil retire le bandeau « réglages réinitialisés », qui le demandait (I016.8). Critère retiré : `EX-014-08` (il
  citait le menu) ; ajouté : `EX-014-09`, le même par la fenêtre.
