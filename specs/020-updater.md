---
id: "020"
titre: "Updater Artifactory : signature ed25519, self-replace, canal bêta"
etape: "7b"
statut: "prévue"
depend_de: ["011", "019"]
suivi: "github#21"
---

# 020 — Updater Artifactory : signature ed25519, self-replace, canal bêta

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
6. le SHA-256 de l'exe téléchargé est égal à `sha256`.

### Clés

- Clés publiques embarquées dans l'exe, **2 emplacements** (courante et suivante) pour permettre une rotation.
- Une clé de développement sert aux tests. `cargo xtask check-release` échoue si elle est embarquée, et le build
  de release aussi (`MCP_PROXY_RELEASE=1`).
- `cargo xtask keygen`, `sign` (clé privée lue dans une variable d'environnement, jamais affichée) et `verify`.

### Application (D12)

- Téléchargement dans `%LOCALAPPDATA%\MCP Proxy\update\<version>\`, avec taille bornée, délais, TLS natif, aucune
  redirection vers une autre origine.
- Après vérification : `self_replace` **tout de suite**. La nouvelle version prend effet au prochain démarrage.
- État `UpdateReady { version, critical }` dans `AppSnapshot` : bouton « Redémarrer pour mettre à jour » dans
  l'en-tête.
- Release **critique** : redémarrage automatique dès qu'aucun appel n'est en cours et qu'aucune connexion ni
  confirmation n'est en attente, avec une notification expliquant qu'il faudra se reconnecter.
- Redémarrer efface les sessions, qui ne vivent qu'en mémoire : c'est attendu.
- Au démarrage : nettoyage des restes (`.__relocated__.exe`, fichiers temporaires, anciens dossiers `update\`).

### Organisation du code

- `mcp_proxy_deploy::update`, indépendant de la plateforme : client HTTP injecté, trait `ExeReplacer` dont
  l'implémentation réelle appelle `self_replace`.
- `FakeArtifactory` dans le testkit : API Storage et fichiers.

## Tests

- **EX-020-01** — Choix de version : stable et bêta, noms invalides ignorés, ordre des préversions, aucune version plus récente.
- **EX-020-02** — Manifeste : valide ; chaque champ altéré ; ligne manquante ; fins de ligne CRLF ; `key-id` inconnu.
- **EX-020-03** — Signature invalide ; version du manifeste différente du dossier ; version identique ; version inférieure
  (retour arrière refusé) ; empreinte différente ; téléchargement tronqué.
- **EX-020-04** — Dans tous les cas d'échec : `ExeReplacer` n'est **jamais** appelé.
- **EX-020-05** — Garde de la clé de développement.
- **EX-020-06** — Indicateur critique : redémarrage différé tant qu'un appel est en cours.
- **EX-020-07** — Bout en bout avec `FakeArtifactory` jusqu'à l'appel du faux `ExeReplacer`.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-020-08** (manuel) — Publier une version supérieure signée sur un Artifactory de test : l'appli la télécharge, le bouton apparaît,
  le redémarrage lance la nouvelle version.
- **EX-020-09** (manuel) — Un `.sig` altéré, ou un exe remplacé : aucune mise à jour, entrée dans le journal.
- **EX-020-10** (manuel) — Un ancien exe republié sous un numéro plus élevé : refusé.
- **EX-020-11** (manuel) — Préversion publiée : ignorée par défaut, proposée après avoir coché « Recevoir les préversions ».

## Hors périmètre

Pipeline de publication (7c). Authenticode (`028-authenticode.md`).
