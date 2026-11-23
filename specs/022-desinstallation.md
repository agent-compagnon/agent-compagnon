---
id: "022"
titre: "Désinstallation propre"
etape: "7d"
statut: "implémentée"
depend_de: ["019"]
suivi: "github#23"
---

# 022 — Désinstallation propre

> Option retenue au QCM (D14).

## Objectif

Que l'appli se retire entièrement, depuis « Applications installées » de Windows, sans droits admin.

## À faire

### Entrée « Applications installées »

- À l'installation (7a) et à chaque mise à jour : `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\MCP Proxy`
  avec `DisplayName`, `DisplayVersion`, `Publisher`, `InstallLocation`, `DisplayIcon`, `UninstallString`
  (`"…\mcp-proxy.exe" --uninstall`), `NoModify`, `NoRepair`, `EstimatedSize`.

### `mcp-proxy.exe --uninstall`

1. Boîte de confirmation, par une API sûre (dialogue de Tauri ou équivalent).
2. Arrêt de l'instance en cours, s'il y en a une.
3. Suppression : raccourci du menu Démarrer, démarrage automatique, clé AUMID, valeur d'exclusion WER (spec 011),
   entrée de désinstallation, dossier
   de données `%LOCALAPPDATA%\MCP Proxy\` (logs, cache, profil WebView, réglages, téléchargements).
4. Entrées ajoutées aux clients LLM (7e) : proposer de les retirer, case cochée par défaut.
5. Suppression de l'exe et du dossier d'installation (`self_replace::self_delete_outside_path`).

- Aucun secret à effacer : ils n'ont jamais été sur disque.
- Chaque étape tolère que l'élément soit déjà absent. Un échec partiel est signalé avec ce qui reste.

Réalisation (amendement du 2026-11-23) :

- **Entrée « Applications installées »** : `DisplayName` et `Publisher` valent « MCP Proxy » (aucun nom d'éditeur
  n'existe dans le projet : la valeur la plus neutre, I022.10), `DisplayVersion` est la marque de version de l'exe en
  place (valeur absente si elle est illisible), `InstallLocation` le dossier d'installation, `DisplayIcon` l'exe,
  `UninstallString` `"<exe installé>" --uninstall`, `NoModify` et `NoRepair` à `1`, `EstimatedSize` la taille de
  l'exe en Kio, arrondie au-dessus. Elle est écrite par l'installation (étape de la transaction, défaite par un retour
  arrière), par la mise à jour de la fenêtre d'installation (idem), par l'updater une fois le nouvel exe en place (au
  mieux, journalisé) et à chaque démarrage de l'exe installé (au mieux : une installation faite ou mise à jour par
  une version antérieure à cette spec la reçoit ainsi) ; elle n'est réécrite que si elle diffère.
- **Qui désinstalle** : seul l'exe installé (même fichier que `…\Programs\MCP Proxy\mcp-proxy.exe`, comparé comme
  à l'installation). Lancé d'ailleurs ou sans installation : boîte de message, rien n'est modifié (I022.2).
- **Fenêtre** : page locale `uninstall.html`, fenêtre `uninstall` de 560 × 700, capacité `uninstall`, une seule
  commande `answer_uninstall`, dans une appli Tauri à part, sans instance unique ni passerelle (comme la fenêtre
  d'installation). Elle dit ce qui sera supprimé (programme et dossier, journaux, réglages, cache, téléchargements,
  profil des fenêtres de connexion, raccourci, démarrage automatique, inscription des notifications, exclusion WER,
  entrée de désinstallation) et ce qui ne le sera pas (entrée d'un client qui lance un autre programme, fichier de
  client illisible, sauvegardes `.bak-…`, diagnostics exportés, exe téléchargé ailleurs ; aucun secret sur disque),
  avec la case « Retirer MCP Proxy des clients LLM », cochée, et les clients concernés. « Désinstaller » rend le
  compte rendu : retiré, retiré à la fermeture de la fenêtre, ce qui reste et pourquoi (échecs en rouge). Fermer la
  fenêtre est refusé pendant le travail. Un échec de la préparation (instance qui ne s'arrête pas…) dit que rien n'a
  été modifié et propose « Réessayer ».
- **Ordre** (I022.3) : (1) l'exe est renommé `mcp-proxy.exe.old` dans son dossier — un client LLM qui relance son
  pont ne trouve plus rien à lancer, donc ne relance pas l'appli ; depuis la spec 026, un pont déjà lancé, qui
  survit à l'arrêt de l'appli, voit l'exe absent et s'arrête en 2 s sans rien relancer — puis (2) l'instance est arrêtée par le canal de
  contrôle de la spec 019 (20 s au plus) ; si elle ne s'arrête pas, l'exe est remis et rien d'autre n'est touché.
  Puis, chaque élément tenté quoi qu'il arrive aux précédents : entrées des clients LLM, valeur `Run`, raccourci,
  clé AUMID, valeur WER, dossier des données ; enfin, **seulement si rien n'a échoué**, l'entrée de désinstallation,
  l'exe (suppression programmée pour la fin du processus) et le dossier d'installation ; sinon l'exe est remis à sa
  place et l'entrée reste, pour relancer la désinstallation une fois la cause levée. Les données WebView2 des
  fenêtres de l'appli (`%LOCALAPPDATA%\com.mcpproxy.app`, I022.5) sont retirées une fois la fenêtre fermée ; un
  échec à ce moment est dit par une boîte de message.
- **Ce qui n'est pas à nous reste** (I022.6) : valeur `Run` qui lance un autre programme, valeur WER autre que le
  DWORD `1`, clé AUMID ou entrée de désinstallation qui a des sous-clés, entrée de désinstallation qui désigne un
  autre exe, entrée `mcp-proxy` d'un client qui lance un autre exe, fichier de client illisible, fichier du dossier
  d'installation qui n'a pas un nom posé par l'installation ou les mises à jour (le dossier reste alors avec lui). Le
  compte rendu les nomme.
- **Garde-fous** (I022.7) : un dossier n'est supprimé récursivement que s'il est exactement
  `<%LOCALAPPDATA%>\<nom attendu>` (`MCP Proxy`, `com.mcpproxy.app`), `%LOCALAPPDATA%` absolu, sans `.` ni `..`, d'au
  moins deux dossiers de profondeur ; un chemin vide, relatif, une racine, un dossier de profil ou un autre nom est
  refusé et rien n'est supprimé. Le parcours ne suit aucun lien : un lien ou un point de jonction rencontré est
  retiré lui-même, jamais sa cible ; un dossier à nous qui est lui-même un lien est laissé. Le dossier d'installation
  n'est jamais supprimé récursivement : ses seuls fichiers connus (`mcp-proxy.exe`, `.old`, `.new`, restes
  `*.__relocated__.exe` et `*.__selfdelete__.exe`) sont retirés un à un, puis le dossier s'il est vide.
- **Fichiers tenus** : le dossier des données est réessayé 10 fois, celui des fenêtres 40 fois, à 250 ms
  d'intervalle (processus WebView2 qui se terminent) ; un reste est nommé avec sa cause, sans chemin de
  l'utilisateur (`%LOCALAPPDATA%\…`).
- **Suppression de l'exe** : `self_replace::self_delete_at` (et non `self_delete_outside_path`, qui part de
  `current_exe()`, chemin d'origine de l'exe déjà renommé : I022.4). L'exe est déplacé dans `%TEMP%`, une copie de
  lui-même attend la fin du processus pour le supprimer, puis disparaît. S'il ne peut pas sortir de son dossier, il
  y est supprimé à la fin du processus et le compte rendu dit que le dossier, alors vide, est à supprimer à la main.
  Si un pont le tient encore à ce moment, il reste dans `%TEMP%` (`.mcp-proxy.<aléa>.__relocated__.exe`) : la
  fenêtre demande de fermer les clients LLM avant de commencer.
- **Clients LLM** : `clients::inspect_all` puis `clients::remove_entry` (sauvegarde, écriture atomique, fichier
  illisible jamais écrit, spec 023) pour chaque entrée `mcp-proxy` dont la commande désigne l'exe installé ; les
  sauvegardes `.bak-…` restent, comme toutes celles de la spec 023 (I022.8).
- **Journal** : la fenêtre de désinstallation n'a pas de passerelle, donc pas de journal ; ce que le code journalise
  ne nomme que des éléments et des genres d'erreur, jamais un chemin.

### Organisation du code

- `mcp_proxy_deploy::uninstall` : plan de désinstallation calculé par une fonction pure ; opérations derrière le
  même trait qu'en 7a. Aucun `unsafe` (D18) : registre par `winreg` (entrée de désinstallation, clé `Run`, clé AUMID,
  exclusion WER), raccourci supprimé comme un fichier, plus de `IShellLinkW`.

Réalisation : `mcp_proxy_deploy::uninstall` (`plan`, `decide`, `Item`, `ClientDecision`, `UninstallPaths`, `prepare`,
`remove`, `remove_window_data`, `Context`, `Report`, `Outcome`, `UninstallError`, `check_owned`, `remove_tree`,
`launch`, `UninstallDialog`, `parse_answer`, `report_view`, `after_close_message`, `UninstallEntry`, `entry_for`,
`record_entry`), `mcp_proxy_deploy::ops` (`PathKind`, `Removal`, `kind`, `remove_link`, `file_len`,
`uninstall_entry`, `set_uninstall_entry`, `remove_uninstall_entry`, `remove_aumid`, `remove_wer_exclusion`,
`delete_running_exe`), `mcp_proxy_deploy::fake` (liens, registre, fichiers tenus). Argument `--uninstall` dans
`gateway::launch`. `app/` : `uninstall.rs` (décision, fenêtre, commande), `system_ops.rs` (registre, `self_replace`),
`update.rs` (entrée réécrite au démarrage). `ui/` : `uninstall.html`, `src/uninstall/`.

## Tests

- **EX-022-01** — Plan complet ; éléments déjà absents ; échec à une étape → les suivantes sont tentées, le rapport liste les
  restes.
- **EX-022-02** — L'entrée de registre porte la bonne version après une mise à jour.
- **EX-022-06** — Garde-fous : un chemin vide, relatif, une racine, un dossier de profil, un autre nom, un `..` ou un
  `%LOCALAPPDATA%` trop court sont refusés sans rien supprimer ; sur de vrais dossiers, aucun lien n'est suivi hors
  de nos dossiers, un dossier à nous qui est un lien est laissé ; les dossiers retirés sont ceux qu'écrivent les
  specs précédentes.
- **EX-022-07** — Préparation : l'exe est mis de côté avant l'arrêt de l'instance ; une instance qui ne s'arrête
  pas, une ancienne version encore tenue ou un exe impossible à mettre de côté ne changent rien (exe remis, ou
  marche à suivre s'il ne peut pas l'être).
- **EX-022-08** — Fin : un dossier tenu est réessayé un nombre borné de fois puis nommé ; un exe qui ne peut être
  supprimé que dans son dossier laisse le dossier à l'utilisateur ; les données de la fenêtre sont retirées après
  sa fermeture, avec un message si elles restent ; sans minutage.
- **EX-022-09** — Clients LLM : avec la case, l'entrée qui lance l'exe installé est retirée avec sauvegarde ; sans
  la case, aucun fichier n'est écrit ; une entrée d'un autre exe et un fichier illisible ne sont jamais écrits et
  sont nommés.
- **EX-022-10** — `--uninstall` reconnu exactement ; seul l'exe installé désinstalle ; réponses de la fenêtre
  strictes, une action à la fois, fermeture refusée pendant le travail ; ni la page ni le journal ne contiennent un
  chemin de l'utilisateur.
- **EX-022-11** — Fenêtre (navigateur, désinstallation simulée) : confirmation, case décochée, compte rendu complet,
  compte rendu avec des restes, échec puis « Réessayer » ; réponses envoyées par sa seule commande ; rien ne déborde,
  en clair et en sombre.
- **EX-022-12** — Contrat : les données et les comptes rendus sérialisés sont `ui/src/uninstall/uninstall.example.json`,
  lu strictement par la page.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-022-03** (manuel) — L'appli apparaît dans « Applications installées » avec la bonne version.
- **EX-022-04** (manuel) — Désinstaller : plus de raccourci, plus de démarrage automatique, plus de dossier d'installation ni de données,
  plus d'entrée dans la liste.
- **EX-022-05** (manuel) — Aucune demande d'élévation.
- **EX-022-13** (manuel) — Désinstaller pendant qu'un client LLM tient un pont : l'appli s'arrête, le client ne
  relance pas l'appli ; noter ce qui reste dans `%TEMP%` et dans le dossier d'installation.
- **EX-022-14** (manuel) — Échec partiel (fichier du dossier des données ouvert par un autre programme) : le compte
  rendu le nomme, MCP Proxy reste installé et la désinstallation se relance depuis « Applications installées ».
- **EX-022-15** (manuel) — `--uninstall` lancé depuis un autre exe, ou sans installation : message, rien n'est
  modifié.
- **EX-022-16** (manuel) — Installation faite par une version antérieure à cette spec, mise à jour par l'updater :
  l'entrée apparaît au premier démarrage de la nouvelle version, avec sa version.

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur) : registre par `winreg`, dialogue par une API
  sûre, raccourci supprimé comme un simple fichier ; la valeur d'exclusion WER posée par l'appli (spec 011) est
  retirée elle aussi. Aucun critère touché.
- **2026-11-23** — Réalisation (I022.1 à I022.12). Écarts au texte : la confirmation est une petite fenêtre de
  l'appli (une boîte de message n'a pas de case) ; l'exe est mis de côté **avant** l'arrêt de l'instance, pour qu'un
  client LLM ne relance pas l'appli pendant la désinstallation ; l'entrée de désinstallation, l'exe et le dossier
  d'installation ne partent que si rien d'autre n'a échoué (sinon MCP Proxy reste installé pour relancer la
  désinstallation) ; `self_replace::self_delete_at` au lieu de `self_delete_outside_path` ; seul l'exe installé
  désinstalle ; seules les entrées des clients qui lancent l'exe installé sont retirées ; ce qui n'est pas à nous
  reste et est nommé. Ajouts trouvés dans le code des specs précédentes : les données WebView2 des fenêtres de
  l'appli (`%LOCALAPPDATA%\com.mcpproxy.app`, posées par Tauri), le raccourci temporaire `MCP Proxy.lnk.tmp`, les
  restes `*.__relocated__.exe` et `*.__selfdelete__.exe`. L'entrée est aussi réécrite à chaque démarrage de l'exe
  installé. Critères ajoutés : `EX-022-06` à `EX-022-12` (automatiques), `EX-022-13` à `EX-022-16` (recette). Aucun
  critère retiré.
- **2026-11-23** — Libellé du journal quand l'appli s'arrête à la demande du canal de contrôle : « arrêt demandé
  par l'installation ou la désinstallation de MCP Proxy », au lieu de « … par l'installation d'une nouvelle
  version … » (spec 019). Raison : la désinstallation emprunte le même canal, qui ne dit pas qui demande l'arrêt ;
  après une désinstallation incomplète, le journal resté sur le disque aurait dit une chose fausse. Aucun critère
  touché.
- **2026-11-23** — Spec 026 : les ponts des clients LLM survivent à l'arrêt de l'appli ; pendant une
  désinstallation, l'exe étant mis de côté avant l'arrêt, un pont le voit absent et s'arrête en 2 s, sans relancer
  l'appli (il tiendrait sinon l'image de l'exe, ce qui laisserait le fichier dans `%TEMP%`). La désinstallation
  elle-même ne change pas ; aucun critère touché.
