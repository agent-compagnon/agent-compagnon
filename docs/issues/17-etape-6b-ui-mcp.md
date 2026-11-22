# [Étape 6b] ui — onglet MCP

> Référence commune : `00-cadrage.md`. Dépend de : étapes 3d et 6a.

## Objectif

Voir d'un coup d'œil quels MCP et quels outils sont exposés, les retrouver par une recherche, et activer ou couper
l'exposition.

## À faire

### Pastilles d'environnements

- Une pastille par environnement, cliquable, qui active ou désactive son **exposition**. L'état vient des réglages
  et du profil.
- **Compteur d'outils exposés**, orange au-delà de `exposed_tools_warning_threshold`, avec une infobulle :
  certains clients plafonnent à 128 outils.

### Bandeau de mise à jour

- Quand un MCP a changé : « Kibana Logs (PROD) a été mis à jour : 2 nouveaux outils », avec « Voir » (filtre le
  tableau sur ce MCP) et « J'ai vu » (`acknowledge_mcp_update`).
- Les outils nouveaux portent une marque « nouveau » et leur politique (confirmation ou interdit).

### Recherche plein texte

- **Insensible aux accents et à la casse** (normalisation NFD puis retrait des diacritiques).
- Porte sur : nom du MCP, environnement, session, politique, état, **et noms et descriptions des outils**.
- Quand la correspondance vient d'un outil, la ligne du MCP se déplie et **montre les outils qui correspondent**,
  le terme surligné.
- Plusieurs mots : tous doivent correspondre.

### Filtre par environnement

- « Tous » ou un seul environnement, chacun avec son nombre de MCP.
- **Indépendant de l'exposition** : on peut filtrer sur un environnement non exposé pour voir ce qu'il contient.

### Tableau

Data Table shadcn-vue sur TanStack Table.

| Colonne | Contenu |
|---|---|
| MCP | Libellé, et « distant » ou « local » |
| Env. | Environnement |
| Session | Session utilisée, avec son point d'état |
| Outils | Nombre d'outils |
| Politique | Politique par défaut (lecture, écriture, destructif), explication en infobulle |
| État | Catalogue, connecté, session requise, injoignable, coupé |
| Exposé | Interrupteur qui coupe ce MCP précis |

- Tri par colonne. Ligne dépliable : liste des outils avec nom exposé, description, catégorie, politique, marque
  « inconnu » ou « modifié ».
- Un interrupteur désactivé indique pourquoi (environnement non exposé).
- Commandes : `set_environment_exposed`, `set_mcp_exposed`, `acknowledge_mcp_update`.

## Tests

- Normalisation : `développeur` trouve `Developpeur`, `PROD` trouve `prod`, `kibána` trouve `Kibana`.
- Recherche sur chaque champ ; correspondance dans une description d'outil → ligne dépliée, outils filtrés ;
  plusieurs mots.
- Filtre par environnement indépendant de l'exposition ; compteurs exacts.
- Pastilles et interrupteurs appellent les bonnes commandes ; interrupteur désactivé quand l'environnement ne l'est
  pas.
- Bandeau : apparition, « Voir », « J'ai vu ».
- Compteur orange au-delà du seuil.
- Playwright : captures de l'onglet (liste, recherche avec outils dépliés, bandeau), en clair et en sombre.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- Désactiver un environnement retire ses outils du client LLM (après rechargement de la liste côté client).
- Couper un MCP retire ses seuls outils.

## Hors périmètre

Édition des politiques depuis l'interface.
