---
id: "013"
titre: "app — notifications Windows et confirmations"
etape: "5c"
statut: "prévue"
depend_de: ["007", "011", "012"]
suivi: "github#14"
---

# 013 — app — notifications Windows et confirmations

## Objectif

Prévenir l'utilisateur quand un LLM attend une connexion, et lui faire valider hors du chat les appels soumis à
confirmation. Sans réponse, c'est un refus.

## À faire

### Identité de l'appli pour les toasts

- Au démarrage : enregistrer l'AUMID **par le registre seulement** (`winreg`), sous
  `HKCU\Software\Classes\AppUserModelId\McpProxy.Desktop` (`DisplayName`, `IconUri`). Idempotent, sans droits
  admin, et sans dépendre du raccourci du menu Démarrer. Plus d'appel à `SetCurrentProcessExplicitAppUserModelID`,
  qui n'existe qu'en `unsafe` (D18) : chaque toast est émis avec cet AUMID explicitement. Les valeurs à écrire sont
  calculées par une fonction pure, dans un crate.

### Service de toasts

- Toasts et état des notifications **par WinRT**, sans `unsafe` : `tauri-winrt-notification` pour l'affichage et
  les boutons (`add_button(texte, action)`, `on_activated(Fn(Option<String>))`, `on_dismissed`), la projection WinRT
  du crate `windows` (feature `UI_Notifications`) pour ce qu'il ne couvre pas : savoir si les toasts sont activés
  (`ToastNotificationManager::CreateToastNotifierWithId(..)?.Setting()?`) et retirer un toast
  (`ToastNotificationManager::History()`, à vérifier à l'implémentation).
- Trois usages :
  - **Connexion demandée** : « Claude Desktop attend une connexion à SSO PROD ». Le clic met la fenêtre de login au
    premier plan.
  - **Confirmation** : voir ci-dessous.
  - **Information** : mise à jour prête, MCP mis à jour, réglages réinitialisés.

### Confirmation

- Contenu : chemin (`PROD / kibana-logs / delete_index`), client, arguments **filtrés**.
- Si les arguments tiennent entièrement dans le toast : boutons « Refuser » et « Autoriser une fois ».
- S'ils seraient tronqués : boutons « Refuser » et « Voir le détail ». On n'approuve jamais ce qu'on n'a pas pu
  lire.
- Le toast reste affiché jusqu'à la décision ou l'échéance (`scenario` rappel). À l'échéance, il est retiré.
- **Toasts désactivés** (Ne pas déranger, stratégie d'entreprise) : pas de toast, la fenêtre de détail s'ouvre
  directement et l'icône passe à l'orange.
- Fenêtre de détail : fenêtre locale, au premier plan. Elle montre le chemin, le client, l'explication de la
  politique, les arguments complets filtrés, le compte à rebours, et les boutons « Refuser » et « Autoriser une
  fois ». La fermer vaut refus.

### `ConfirmationBroker`

- `PendingConfirmations` : registre des demandes en attente, dans un crate, testé sous Linux. Une demande se résout
  une seule fois, par la première réponse reçue (bouton du toast, fenêtre de détail, bandeau de l'interface).
  Échéance → `TimedOut`. Tout cas imprévu → refus.
- L'appli se contente d'afficher et de transmettre la réponse.
- Les demandes en attente figurent dans `AppSnapshot` : bandeau orange dans l'interface, icône orange.

### Raccordement

- `AuthUi::notify_login_needed` → toast « Connexion demandée ».
- `ConfirmationBroker` branché dans la chaîne de middlewares à la place du provisoire de 5a.

## Tests (sous Linux)

- **EX-013-01** — `PendingConfirmations` : approbation, refus, échéance, double réponse (la seconde est ignorée), demande inconnue,
  arrêt de l'appli avec des demandes en attente (toutes refusées).
- **EX-013-02** — Règle « tient dans le toast » : arguments courts, longs, multi-lignes.
- **EX-013-03** — Les textes du toast et de la fenêtre sont construits à partir d'arguments déjà filtrés (valeur sentinelle).
- **EX-013-11** — Valeurs de registre de l'AUMID (clé, `DisplayName`, `IconUri` pointant l'exe installé) calculées
  pour un chemin d'installation donné.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-013-04** (manuel) — Un appel qui exige une connexion affiche le toast ; le clic amène la fenêtre de login devant.
- **EX-013-05** (manuel) — Arguments courts : « Autoriser une fois » laisse passer l'appel ; « Refuser » renvoie le refus au LLM.
- **EX-013-06** (manuel) — Arguments longs : le toast propose « Voir le détail » ; la fenêtre montre tout ; la fermer refuse.
- **EX-013-07** (manuel) — Sans réponse pendant 30 s : refus, toast retiré.
- **EX-013-08** (manuel) — Avec « Ne pas déranger » activé : la fenêtre de détail s'ouvre seule.
- **EX-013-09** (manuel) — L'icône est orange tant qu'une demande attend, puis redevient neutre.
- **EX-013-10** (manuel) — Sans raccourci dans le menu Démarrer (exe lancé depuis un autre dossier), un toast
  s'affiche avec le nom « MCP Proxy » et son icône : l'AUMID enregistré par le registre suffit.

## Hors périmètre

Canal elicitation (fait en 3c). Bandeaux de l'interface (6a).

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur). Toasts et état des notifications par WinRT ;
  AUMID enregistré par le registre (`winreg`) et passé explicitement à chaque toast ; plus de
  `SetCurrentProcessExplicitAppUserModelID`. Aucun critère retiré ; ajoutés : `EX-013-10` et `EX-013-11`.
