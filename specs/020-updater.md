---
id: "020"
titre: "Updater Artifactory : signature ed25519, remplacement de l'exe, canal bêta"
etape: "7b"
statut: "implémentée"
depend_de: ["011", "019"]
suivi: "github#21"
---

# 020 — Updater Artifactory : signature ed25519, remplacement de l'exe, canal bêta

> Le canal bêta est une option retenue au QCM (D14).

## Objectif

Mettre l'exe à jour tout seul depuis un dépôt Artifactory générique en release, où aucun fichier n'est modifiable,
sans jamais installer un binaire qui ne serait pas authentique, intact et plus récent.

## À faire

### Dépôt

```
mcp-proxy/<semver>/mcp-proxy.exe
mcp-proxy/<semver>/mcp-proxy.exe.sig
```

Pas de `latest.json`. Lecture anonyme (D11).

### Découverte

- **API Storage** : `GET {base_url}/api/storage/{repository}/{path}` ; lire `children`, garder les dossiers dont le
  nom est un semver valide, ignorer le reste.
- Candidats : versions strictement supérieures à la version courante.
- **Canal stable** : les préversions (`1.4.0-rc.1`) sont ignorées. **Canal bêta** (`UserSettings::beta_channel`,
  case « Recevoir les préversions ») : elles sont admises. Choisir la plus haute selon la précédence semver.
- Vérification au démarrage (après une minute) puis toutes les `check_interval_hours`. Un échec n'est visible que
  dans le journal.
- Réalisation (amendement du 2026-11-22) : seuls les enfants `folder: true` dont le nom est un semver sous sa seule
  écriture comptent ; la version retenue doit dépasser la version en cours, celle déjà mise en place par cette
  exécution et celle lue dans l'exe installé (marque de version, spec 019) ; seule la plus haute est essayée. Cocher
  ou décocher « Recevoir les préversions » lance une vérification tout de suite. La case est dans le **menu de
  l'icône**, à côté de « Démarrer avec Windows », grisée quand l'updater ne tourne pas ; elle s'enregistre comme les
  autres réglages scellés. L'updater ne tourne que dans l'exe installé et avec au moins une clé embarquée ; sans clé,
  le journal le dit une fois.

### Fichier `.sig`

Texte UTF-8, fins de ligne LF :

```
mcp-proxy-signature v1
version: 1.4.0
sha256: <64 caractères hexadécimaux>
critical: false
key-id: <8 caractères hexadécimaux>
signature: <base64>
```

- `signature` est la signature Ed25519 de toutes les lignes qui précèdent, fins de ligne comprises.
- `key-id` : 4 premiers octets du SHA-256 de la clé publique.
- Signer la version et l'empreinte, et pas seulement l'exe, empêche de republier un ancien exe valide sous un
  numéro plus élevé.

### Vérification, avant tout remplacement

Dans cet ordre ; le moindre échec abandonne la mise à jour et supprime les fichiers téléchargés :

1. le manifeste est bien formé ;
2. `key-id` désigne une des clés publiques embarquées ;
3. la signature Ed25519 est valide (vérification stricte) ;
4. `version` est égale au nom du dossier ;
5. `version` est strictement supérieure à la version courante ;
6. le SHA-256 de l'exe téléchargé est égal à `sha256` ;
7. la marque de version de l'exe téléchargé (spec 019) est égale à `version` (amendement du 2026-11-22).

Réalisation : les étapes 1 à 5 portent sur le `.sig` seul, téléchargé et vérifié **avant** l'exe ; la « version
courante » de l'étape 5 est le plus haut de la version en cours, de la version déjà mise en place et de celle de
l'exe installé (un exe installé de version illisible n'est jamais remplacé). Lecture stricte du manifeste : 1 024
octets au plus, aucun `\r`, six lignes terminées par LF, une seule écriture par champ (hexadécimal minuscule,
base64 canonique, semver sous sa seule écriture). Les deux fichiers vont dans `update\<version>\`, retirés en entier
quel que soit le résultat.

### Clés

- Clés publiques embarquées dans l'exe, **2 emplacements** (courante et suivante) pour permettre une rotation.
- Une clé de développement sert aux tests. `cargo xtask check-release` échoue si elle est embarquée, et le build
  de release aussi (`MCP_PROXY_RELEASE=1`).
- `cargo xtask keygen`, `sign` (clé privée lue dans une variable d'environnement, jamais affichée) et `verify`.
- Réalisation (amendement du 2026-11-22) : les emplacements sont `crates/deploy/keys/current.pub` et `next.pub`,
  lus par le `build.rs` du crate `deploy` (vides tant que la spec 021 n'a pas créé la clé de release : l'exe ne se met
  alors jamais à jour). La clé de développement (graine `DEV_SIGNING_SEED` du testkit, clé publique `keys/dev.pub`)
  n'est **jamais** embarquée : **tout** build échoue si un emplacement la contient, l'exe l'écarterait quand même, et
  les tests passent leurs clés à l'updater. Le build de release et `check-release` exigent en plus une clé courante ;
  `check-release --exe <exe>` vérifie aussi qu'un exe construit contient la clé courante et pas les octets de la clé
  de développement. Les commandes passent par le binaire `update-tool` du crate `deploy` ; `keygen --out <fichier>`
  écrit la clé privée dans un fichier créé sans écraser et n'affiche que la clé publique ; `sign` lit la clé dans
  `ED25519_PRIVATE_KEY` (ou `--key-env`), prend la version dans la marque de l'exe et refuse un `--version`
  différent, `--critical true|false` ; `verify` contrôle signature, empreinte et marque avec `--public-key` ou les
  clés embarquées.

### Application (D12)

- Téléchargement dans `%LOCALAPPDATA%\MCP Proxy\update\<version>\`, avec taille bornée, délais, TLS natif, aucune
  redirection vers une autre origine.
- Après vérification : remplacement **tout de suite**. La nouvelle version prend effet au prochain démarrage.
- État `UpdateReady { version, critical }` dans `AppSnapshot` : bouton « Redémarrer pour mettre à jour » dans
  l'en-tête.
- Release **critique** : redémarrage automatique dès qu'aucun appel n'est en cours et qu'aucune connexion ni
  confirmation n'est en attente, avec une notification expliquant qu'il faudra se reconnecter.
- Redémarrer efface les sessions, qui ne vivent qu'en mémoire : c'est attendu.
- Au démarrage : nettoyage des restes (`.__relocated__.exe`, fichiers temporaires, anciens dossiers `update\`).

Réalisation (amendement du 2026-11-22) :

- **Remplacement** : le mécanisme de la spec 019 (I019.7), pas `self_replace` : copie vers `mcp-proxy.exe.new` dans
  le dossier d'installation, nouvelle empreinte de la copie (elle doit être l'exe vérifié), exe installé renommé en
  `mcp-proxy.exe.old` (un exe en cours, tenu par l'appli ou par un pont, se renomme mais ne se remplace pas), `.new`
  renommé en `mcp-proxy.exe` ; un échec remet l'ancien exe et retire la copie. `.old` reste jusqu'au démarrage
  suivant ; un `.old` encore tenu par un pont d'une version antérieure est renommé
  `mcp-proxy.exe.old-<n>.__relocated__.exe` (nom de reste, retiré par le nettoyage une fois libre, spec 026) ; s'il ne
  peut pas l'être non plus, le remplacement échoue et la vérification suivante réessaie. Une fois le nouvel exe en place, l'entrée « Applications installées » (spec 022) prend sa
  version, au mieux (un échec est journalisé, le remplacement n'est pas défait) ; elle est de toute façon réécrite au
  démarrage suivant de l'exe installé, avec le nettoyage des restes.
- **Téléchargement** : client à part, anonyme (ni identifiant, ni cookie, ni `Referer`), TLS natif, `https` (`http`
  vers la boucle locale seulement), redirections suivies dans la même origine seulement (5 au plus), 1 Mio pour la
  liste, 1 024 octets pour le `.sig`, 256 Mio pour l'exe, délais bornés. Erreurs et journal sans adresse, chemin ni
  contenu de réponse.
- **Instantané** : `update` vaut `{ version, critical }` ou `null`. Une version mise en place après une critique
  reste critique. Notifications : « Mise à jour prête » ou « Mise à jour critique » (redémarrage automatique,
  reconnexion à prévoir), une fois par version.
- **Redémarrage** : « Redémarrer pour mettre à jour » demande d'abord confirmation (les connexions seront fermées),
  puis envoie `restart_for_update` (refusée sans mise à jour en place) ; l'appli quitte comme par « Quitter », et une
  fois sa boucle d'événements terminée (instance unique libérée) et ses tâches arrêtées (pipe fermé), lance l'exe
  installé. Depuis la spec 026, les ponts des clients LLM (ancienne version, image `mcp-proxy.exe.old`) attendent la
  nouvelle instance sans la relancer eux-mêmes (marque `arret-volontaire`) et s'y reconnectent : le client n'est pas
  à redémarrer, seules les sessions sont à rouvrir.
- **Release critique** : après la notification, au moins une minute de délai, puis attente qu'aucun appel ne soit en
  cours (appels comptés de leur arrivée à leur réponse, ceux qui attendent une connexion ou une confirmation
  compris), qu'aucune connexion ne soit demandée et qu'aucune confirmation n'attende ; puis redémarrage.
- **Nettoyage au démarrage** : `mcp-proxy.exe.old`, `mcp-proxy.exe.new`, `*.__relocated__.exe` (dont les `.old` mis
  de côté, spec 026),
  `*.__selfdelete__.exe` du dossier d'installation et tout le contenu de `update\` ; le journal ne donne que des
  nombres.

### Organisation du code

- `mcp_proxy_deploy::update`, indépendant de la plateforme : client HTTP injecté, trait `ExeReplacer` dont
  l'implémentation réelle renomme l'exe installé (spec 019).
- `FakeArtifactory` dans le testkit : API Storage et fichiers.

Réalisation : `mcp_proxy_deploy::update` (`Updater`, `UpdaterDeps`, `Source`, `UpdateError`, `CheckOutcome`,
`UpdateReady`, `Channel`, `parse_listing`, `choose`, `Manifest`, `KeyRing`, `KeyId`, `Fetch`, `AnonymousClient`,
`ExeReplacer`, `RenameReplacer`, `VerifiedExe`, `UpdatePaths`, `clean_leftovers`, `release`, `slots`),
`mcp_proxy_deploy::fake::FakeReplacer`, binaire `update-tool`. `mcp_proxy_gateway::self_update` (`SelfUpdateDeps`,
`Pause`, `Restarter`, `Activity`, `ActivityProbe`, `CallsInFlight`, `restart_when_idle`, `update_information`,
`Gateway::start_self_update`, `restart_for_update`, `set_beta_channel`), `view::UpdateReadyView`. `app/` :
`update.rs` (client, remplaceur, redémarrage, notifications), entrée « Recevoir les préversions » du menu de
l'icône, commande `restart_for_update`. Testkit : `FakeArtifactory`, `Served`, `DEV_SIGNING_SEED`. `ui/` : en-tête
et son dialogue.

## Tests

- **EX-020-01** — Choix de version : stable et bêta, noms invalides ignorés, ordre des préversions, aucune version plus récente.
- **EX-020-02** — Manifeste : valide ; chaque champ altéré ; ligne manquante ; fins de ligne CRLF ; `key-id` inconnu.
- **EX-020-03** — Signature invalide ; version du manifeste différente du dossier ; version identique ; version inférieure
  (retour arrière refusé) ; empreinte différente ; téléchargement tronqué.
- **EX-020-04** — Dans tous les cas d'échec : `ExeReplacer` n'est **jamais** appelé.
- **EX-020-05** — Garde de la clé de développement.
- **EX-020-06** — Indicateur critique : redémarrage différé tant qu'un appel est en cours.
- **EX-020-07** — Bout en bout avec `FakeArtifactory` jusqu'à l'appel du faux `ExeReplacer`.
- **EX-020-12** — Téléchargement anonyme : aucun en-tête d'authentification, de cookie ni `Referer` ; HTTPS seulement
  (HTTP vers la boucle locale seulement) ; redirection suivie dans la même origine seulement, chaîne bornée ; taille
  bornée, annoncée ou reçue ; coupure détectée.
- **EX-020-13** — Remplacement par renommage (faux système, vrais fichiers) : ordre des opérations ; un échec à chaque
  étape laisse l'exe installé tel quel et retire la copie ; une copie différente de l'exe vérifié n'est jamais mise
  en place ; un ancien exe impossible à remettre est nommé.
- **EX-020-14** — Nettoyage au démarrage : `.old`, `.new`, `.__relocated__.exe`, `.__selfdelete__.exe` et tout
  `update\` sont retirés, le reste est intact, le journal ne donne que des nombres.
- **EX-020-15** — `keygen`, `sign`, `verify` : cycle complet ; la clé privée n'apparaît jamais dans ce que l'outil
  affiche, erreurs comprises ; `keygen` n'écrase rien et crée un fichier lisible du seul utilisateur ; `sign` contrôle
  la version de l'exe ; arguments stricts.
- **EX-020-16** — Planification (horloge injectée) : première vérification après une minute, puis toutes les
  `check_interval_hours` ; cocher « Recevoir les préversions » relance une vérification ; un échec n'apparaît que dans
  le journal, sans adresse ni chemin.
- **EX-020-17** — État `UpdateReady { version, critical }` dans l'instantané (contrat avec l'interface), notification
  de mise à jour prête ; la commande de redémarrage est refusée sans mise à jour prête.
- **EX-020-18** — Interface (navigateur, backend simulé) : bouton « Redémarrer pour mettre à jour » actif, dialogue qui
  prévient que les connexions seront fermées, état critique ; en clair et en sombre.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-020-08** (manuel) — Publier une version supérieure signée sur un Artifactory de test : l'appli la télécharge, le bouton apparaît,
  le redémarrage lance la nouvelle version.
- **EX-020-09** (manuel) — Un `.sig` altéré, ou un exe remplacé : aucune mise à jour, entrée dans le journal.
- **EX-020-10** (manuel) — Un ancien exe republié sous un numéro plus élevé : refusé.
- **EX-020-11** (manuel) — Préversion publiée : ignorée par défaut, proposée après avoir coché « Recevoir les préversions ».
- **EX-020-19** (manuel) — Release critique : notification qui annonce le redémarrage et la reconnexion ; redémarrage
  automatique seulement une fois les appels, connexions et confirmations en cours terminés.
- **EX-020-20** (manuel) — Mise à jour pendant qu'un client LLM tient l'exe installé (pont) : le remplacement réussit,
  le client continue, et `mcp-proxy.exe.old` disparaît à un démarrage suivant, une fois le pont fermé.

## Hors périmètre

Pipeline de publication (7c). Authenticode (`028-authenticode.md`).

## Amendements

- **2026-11-22** — Réalisation (I020.1 à I020.13). Écarts au texte : remplacement par renommage (mécanisme de la
  spec 019) au lieu de `self_replace`, d'où le titre ; étape 7 de vérification ajoutée (marque de version de l'exe) ;
  le `.sig` est vérifié avant le téléchargement de l'exe ; la clé de développement n'est jamais embarquée, même hors
  release (tout build échoue si un emplacement la contient), et le build de release exige une clé courante ; case
  « Recevoir les préversions » dans le menu de l'icône, qui lance une vérification ; délai d'au moins une minute
  avant le redémarrage automatique d'une release critique ; dialogue de confirmation avant « Redémarrer pour mettre à
  jour ». Critères ajoutés : `EX-020-12` à `EX-020-18` (automatiques), `EX-020-19` et `EX-020-20` (recette). Aucun
  critère retiré. Raison : points laissés ouverts par le texte, tranchés par l'option la plus sûre, et `self_replace`
  incompatible avec un exe tenu par les ponts des clients LLM (I019.7).
- **2026-11-23** — Spec 022 : après le remplacement, l'entrée « Applications installées » prend la version mise en
  place ; au démarrage de l'exe installé, avec le nettoyage des restes, l'entrée est réécrite si elle diffère (une
  installation mise à jour par une version antérieure à la spec 022 la reçoit ainsi). Prouvé par `EX-022-02`. Aucun
  critère touché.
- **2026-11-23** — Spec 026 : le redémarrage pour une mise à jour (bouton ou release critique) écrit, comme tout arrêt
  volontaire, la marque `arret-volontaire` ; les ponts des clients LLM se reconnectent à la nouvelle version sans
  relance du client. Aucun critère touché ; la recette `EX-020-20` ne demande plus de relancer le serveur MCP du
  client.
- **2026-11-23** — Spec 026 : un `mcp-proxy.exe.old` encore exécuté par un pont ne fait plus échouer le remplacement :
  il est renommé `mcp-proxy.exe.old-<n>.__relocated__.exe`, retiré par le nettoyage au démarrage une fois libre.
  Raison : les ponts résilients vivent autant que leur client, et chaque mise à jour suivante aurait échoué. Prouvé
  par `EX-026-20` ; aucun critère de la 020 touché (`EX-020-13` inchangé : un `.old` tenu par un autre programme fait
  toujours échouer l'étape).
