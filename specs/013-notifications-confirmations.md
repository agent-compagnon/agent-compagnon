---
id: "013"
titre: "app — notifications Windows et confirmations"
etape: "5c"
statut: "implémentée"
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
  `HKCU\Software\Classes\AppUserModelId\McpProxy.Desktop` (`DisplayName` « MCP Proxy », `IconUri`). Idempotent, sans
  droits admin, et sans dépendre du raccourci du menu Démarrer. Plus d'appel à `SetCurrentProcessExplicitAppUserModelID`,
  qui n'existe qu'en `unsafe` (D18) : chaque toast est émis avec cet AUMID explicitement. Les valeurs à écrire sont
  calculées par une fonction pure, dans un crate (`mcp_proxy_gateway::notify::aumid_registration`).
- `IconUri` désigne un fichier image : l'icône PNG de l'appli est écrite (seulement si elle a changé) dans
  `%LOCALAPPDATA%\MCP Proxy\notification-icon.png`, et `IconUri` pointe ce fichier.
- Un enregistrement qui échoue rend les toasts indisponibles (un toast pourrait être accepté par Windows sans être
  affiché) : les confirmations passent alors par la fenêtre de détail.
- Au démarrage, les toasts laissés par une exécution précédente sont retirés du centre de notifications ; à l'arrêt,
  tous ceux de l'appli.

### Service de toasts

- Toasts et état des notifications **par la projection WinRT du crate `windows`** (features `Foundation`,
  `Data_Xml_Dom`, `UI_Notifications`), sans `unsafe` : le contenu est construit par le DOM d'un `XmlDocument` (aucun
  texte collé dans du XML), le toast porte une étiquette, un groupe, une échéance (`ExpirationTime`) et est suivi par
  ses événements `Activated`, `Dismissed` et `Failed`. Toasts activés si `ToastNotifier::Setting` vaut `Enabled`
  **et** `ToastNotificationManager::GetDefault()?.NotificationMode()` vaut `Unrestricted` (« Ne pas déranger » donne
  `PriorityOnly` ou `AlarmsOnly`) ; une valeur illisible vaut « désactivés ». Retrait :
  `ToastNotifier::Hide` puis `ToastNotificationManager::History()?.RemoveGroupedTagWithId(..)`.
  `tauri-winrt-notification` n'est pas utilisé (voir Amendements).
- Trois usages :
  - **Connexion demandée** : « Claude Desktop attend une connexion à SSO PROD » (nom du client tiré de
    `clientInfo.name`, filtré). Le clic met la fenêtre de login (ou de saisie Basic) de la session au premier plan.
    Sans toasts, rien : la fenêtre de login, toujours visible, suffit.
  - **Confirmation** : voir ci-dessous.
  - **Information** : réglages réinitialisés (au démarrage), MCP mis à jour (outils changés) ; « mise à jour prête »
    a son texte et sera émis par la spec 020. Le clic ouvre la fenêtre principale. Sans toasts, rien.

### Confirmation

- Contenu : chemin (`PROD / kibana-logs / delete_index`), client, arguments **filtrés**.
- Si les arguments tiennent entièrement dans le toast : boutons « Refuser » et « Autoriser une fois ».
- S'ils seraient tronqués : boutons « Refuser » et « Voir le détail ». On n'approuve jamais ce qu'on n'a pas pu
  lire.
- Règle « tient dans le toast » (fonction pure) : chemin sur 2 lignes au plus, client sur 1 ligne, arguments
  (`clé: valeur JSON`, séparés par des virgules) sur 3 lignes au plus, pour une ligne d'environ 30 caractères
  ordinaires (largeur pondérée : étroits 1, ordinaires 2, larges 3, 60 unités par ligne) ; tout caractère hors ASCII
  imprimable et lettres Latin-1 (saut de ligne, caractère de contrôle, invisible ou bidirectionnel, autre écriture)
  dans le chemin, le client, une clé ou une valeur : ne tient pas.
- Un clic sur le corps du toast ouvre la fenêtre de détail ; il n'approuve jamais.
- Le toast reste affiché jusqu'à la décision ou l'échéance (`scenario` rappel). À l'échéance, il est retiré.
- **Toasts désactivés** (Ne pas déranger, stratégie d'entreprise, AUMID non enregistré, état illisible) : pas de
  toast, la fenêtre de détail s'ouvre directement et l'icône passe à l'orange. Un toast refusé à l'affichage ou
  signalé en échec par Windows est remplacé par la fenêtre de détail. Si la fenêtre ne peut pas s'ouvrir non plus :
  refus (« confirmation impossible, aucun moyen de la demander »).
- Fenêtre de détail : fenêtre locale (`confirm.html`, étiquette `confirm-<n>`), au premier plan (toujours devant).
  Elle montre le chemin, le client, l'explication de la politique, les arguments complets filtrés (JSON indenté,
  caractères invisibles rendus visibles), le compte à rebours, et les boutons « Refuser » et « Autoriser une fois ».
  La fermer vaut refus. Sa capacité `confirm` ne donne que la commande `answer_confirmation` aux fenêtres
  `confirm-*`, contexte local seulement ; la commande ne répond rien et lit la demande dans l'étiquette de la
  fenêtre, jamais dans la page. Elle ne reçoit que sa demande, par un script d'initialisation.

### `ConfirmationBroker`

- `PendingConfirmations` : registre des demandes en attente, dans un crate, testé sous Linux. Une demande se résout
  une seule fois, par la première réponse reçue (bouton du toast, fenêtre de détail). Échéance → `TimedOut`. Tout
  cas imprévu → refus : approbation venue d'un toast qui ne l'offrait pas, activation portant d'autres arguments,
  toast balayé ou envoyé au centre de notifications, réponse de la page autre que `"approve"`, arrêt de l'appli.
  Une demande abandonnée par la chaîne (délai de la chaîne, annulation) est oubliée et son toast retiré.
- `NotificationBroker` (implémente `ConfirmationBroker`) refiltre chaque texte avant de l'afficher (défense en
  profondeur), choisit toast ou fenêtre, et retire toast et fenêtre quand la demande se termine, quelle qu'en soit
  l'issue.
- L'appli se contente d'afficher et de transmettre la réponse (`ConfirmationPresenter`).
- Les demandes en attente figurent dans `AppSnapshot` (`pendingConfirmations`, état « attention », infobulle) et
  chaque changement est signalé : bandeau orange dans l'interface (spec 016), icône orange.

### Raccordement

- `AuthUi::notify_login_needed` → toast « Connexion demandée ». `AccessReason::ToolCall` porte le nom du client,
  filtré par la chaîne.
- `ConfirmationBroker` branché dans la chaîne de middlewares à la place du provisoire de 5a
  (`UnavailableConfirmation`, supprimé) ; `RuntimeDeps` reçoit le `ConfirmationPresenter`.

## Tests (sous Linux)

- **EX-013-01** — `PendingConfirmations` : approbation, refus, échéance, double réponse (la seconde est ignorée), demande inconnue,
  arrêt de l'appli avec des demandes en attente (toutes refusées).
- **EX-013-02** — Règle « tient dans le toast » : arguments courts, longs, multi-lignes.
- **EX-013-03** — Les textes du toast et de la fenêtre sont construits à partir d'arguments déjà filtrés (valeur sentinelle).
- ~~**EX-013-11** — Valeurs de registre de l'AUMID (clé, `DisplayName`, `IconUri` pointant l'exe installé) calculées
  pour un chemin d'installation donné.~~ (retiré le 2026-11-22 : `IconUri` désigne un fichier image, voir
  `EX-013-12`)
- **EX-013-12** — Valeurs de registre de l'AUMID (clé, `DisplayName`, `IconUri` pointant l'icône PNG du dossier des
  données) calculées pour un dossier des données donné ; l'icône n'est réécrite que si elle a changé.
- **EX-013-13** — Une demande en attente figure dans `AppSnapshot` (nombre, état « attention », infobulle) et un
  changement est signalé quand elle se termine ; sans toasts, la fenêtre de détail s'ouvre seule et la fermer
  refuse l'appel.
- **EX-013-14** — Page de la fenêtre de détail (`ui/`) : chemin, client, politique et arguments complets affichés,
  compte à rebours, une seule réponse, plus d'autorisation après l'échéance, données lues de l'appli seulement et
  réponse envoyée à sa seule commande.

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
- **EX-013-15** (manuel) — Toast balayé (croix) ou clic sur son corps : le premier refuse, le second ouvre la
  fenêtre de détail sans rien approuver ; un toast resté dans le centre de notifications après l'échéance n'y est
  plus ; une information (« MCP mis à jour ») s'affiche et son clic ouvre la fenêtre principale.

## Hors périmètre

Canal elicitation (fait en 3c). Bandeaux de l'interface (6a), y compris une réponse depuis le bandeau : elle devra
afficher les arguments complets avant d'offrir l'autorisation.

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur). Toasts et état des notifications par WinRT ;
  AUMID enregistré par le registre (`winreg`) et passé explicitement à chaque toast ; plus de
  `SetCurrentProcessExplicitAppUserModelID`. Aucun critère retiré ; ajoutés : `EX-013-10` et `EX-013-11`.
- **2026-11-22** — Implémentation (I013.1 à I013.12). Lecture des sources de `tauri-winrt-notification` 0.8.1 :
  `Toast::show` crée le `ToastNotification` en interne et ne le rend pas, ne pose ni étiquette ni groupe (sauf avec
  une barre de progression), ne relaie pas l'événement `Failed` et ne règle pas d'échéance : impossible d'y retirer
  un toast ni de savoir qu'il a échoué. Les toasts sont donc construits directement par la projection WinRT du
  crate `windows` (mêmes appels, sûrs, que ce crate fait en interne). « Ne pas déranger » ne se lit pas dans
  `Setting` mais dans `NotificationMode`. `IconUri` désigne un fichier image, pas l'exe : `EX-013-11` est retiré et
  remplacé par `EX-013-12`. Ajoutés : `EX-013-13` (instantané), `EX-013-14` (page de la fenêtre de détail),
  `EX-013-15` (manuel : toast balayé, clic sur le corps, centre de notifications, information).
  `AccessReason::ToolCall` porte le nom du client ; `UnavailableConfirmation` est supprimé.
