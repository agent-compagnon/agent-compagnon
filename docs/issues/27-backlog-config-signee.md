# [Backlog] Config signée téléchargée depuis Artifactory

> Référence commune : `00-cadrage.md`. **Prévu, à ne pas implémenter pour l'instant.**

## Pourquoi

Ajouter un MCP, changer une URL ou une politique oblige aujourd'hui à recompiler et republier l'exe. Une config
signée, téléchargée, éviterait cela.

## Ce qui est déjà prévu

- `ConfigSource` (étape 1) : la config embarquée est une implémentation parmi d'autres possibles.
- `schema_version` dans la config.
- L'infrastructure de signature de l'updater (7b) : clés publiques embarquées, manifeste signé, vérification avant
  usage.

## Esquisse

- Dépôt : `mcp-proxy-config/<n>/config.toml` et `config.toml.sig`, `<n>` entier croissant. Découverte par l'API
  Storage, comme pour l'exe.
- Signature Ed25519 d'un manifeste (`numéro`, `sha256`, `schema_version` minimal et maximal de l'exe). Clé
  **distincte** de celle des mises à jour.
- Vérifications : signature, numéro strictement croissant (pas de retour arrière), compatibilité de schéma,
  validation stricte complète.
- En cas d'échec, à n'importe quelle étape : la config embarquée reste en vigueur.
- La config téléchargée est conservée localement **avec sa signature**, et revérifiée à chaque démarrage.

## Questions à trancher avant de l'implémenter

- Une config distante peut-elle assouplir une politique de la config embarquée, ou seulement durcir et ajouter ?
- Qui signe, et avec quel circuit de validation ?
- Faut-il pouvoir épingler un poste sur une config donnée ?
- Comment l'utilisateur est-il informé qu'un MCP ou une politique a changé ?
