# [Étape 7f] Auto-diagnostic (`doctor`)

> Référence commune : `00-cadrage.md`. Dépend de : étapes 3e, 5a, 7a, 7b. Option retenue au QCM (D14).

## Objectif

Qu'un utilisateur ou le support sache en quelques secondes pourquoi « ça ne marche pas », sans lire de logs.

## À faire

### Trois accès

- `mcp-proxy.exe doctor` : rapport texte et code de sortie (0 tout va bien, 1 avertissements, 2 échecs). L'exe
  étant en sous-système fenêtré, se rattacher à la console parente pour afficher.
- Entrée « Auto-diagnostic » dans le menu de l'en-tête : même rapport dans un dialogue, avec « Copier ».
- Le rapport est joint à l'export de diagnostic (3e).

### Contrôles

Chaque contrôle rend `ok`, `avertissement` ou `échec`, un message, et **quoi faire** pour corriger.

| Contrôle | Détail |
|---|---|
| Config | Chargée et valide |
| WebView2 | Runtime présent, version suffisante pour le mode InPrivate |
| Installation | Exe à l'emplacement attendu, dossier inscriptible, raccourci et AUMID présents |
| Notifications | Toasts activés pour l'appli |
| Appli | Instance en cours, pipe joignable |
| Réglages | Sceau valide (5d) |
| Artifactory | API Storage joignable, dernière version disponible |
| Clé de mise à jour | Ce n'est pas la clé de développement |
| Clients LLM | Détectés, configurés ou non (7e) |
| MCP | Pour chaque instance exposée : résolution de nom et TLS de son origine. **Aucun identifiant n'est envoyé** ; toute réponse HTTP compte comme joignable |
| Horloge | Écart manifeste avec l'heure d'un serveur (les échéances des jetons en dépendent) |

- Aucun secret dans le rapport. Il passe par le filtre comme le reste.

### Organisation du code

- `mcp_proxy_deploy::doctor` : trait `Check`, sondes de plateforme derrière des traits, rendu texte et structuré.

## Tests

- Chaque contrôle dans ses trois issues, avec de fausses sondes.
- Code de sortie selon le pire résultat.
- Le contrôle MCP n'envoie aucun en-tête d'authentification (faux serveur qui enregistre les requêtes).
- Rapport sans secret (valeur sentinelle).

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- `mcp-proxy.exe doctor` dans un terminal affiche le rapport.
- Désactiver les notifications de l'appli dans Windows : le contrôle passe en avertissement avec la marche à
  suivre.
- Le dialogue de l'interface affiche le même rapport.
