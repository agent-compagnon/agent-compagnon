---
id: "017"
titre: "ui — onglet MCP"
etape: "6b"
statut: "en cours"
depend_de: ["008", "016"]
suivi: "github#18"
---

# 017 — ui — onglet MCP

## Objectif

Voir d'un coup d'œil quels MCP et quels outils sont exposés, les retrouver par une recherche, et activer ou couper
l'exposition.

## À faire

### Pastilles d'environnements

- Une pastille par environnement, cliquable, qui active ou désactive son **exposition**. L'état vient des réglages
  et du profil.
- **Compteur d'outils exposés**, orange au-delà de `exposed_tools_warning_threshold`, avec une infobulle :
  certains clients plafonnent à 128 outils.
- Réalisation : pastille pleine pour un environnement exposé, en pointillé sinon, icône de bouclier pour un
  environnement sensible ; infobulle avec la description de l'environnement et l'effet du clic. Exposer est un geste
  de l'utilisateur dans les limites de son profil : **sans profil choisi, rien ne peut être exposé** (pastilles
  inactives, « Choisissez d'abord un profil. », commande refusée) ; dans un environnement exposé, les règles du
  profil actif et les verrous de la config s'appliquent, et la colonne Politique les montre. **Exposer un
  environnement sensible demande une confirmation** (« Exposer PROD aux clients LLM ? ») ; retirer un environnement
  n'en demande jamais. Le compteur est orange strictement au-delà du seuil (`Registry::over_threshold`) ; l'alerte
  `toolThreshold` de l'instantané n'est pas répétée en bandeau. Les alertes d'information (`catalogCache`) sont des
  bandeaux noir et blanc en tête de l'onglet.

### Bandeau de mise à jour

- Quand un MCP a changé : « Kibana Logs (PROD) a été mis à jour : 2 nouveaux outils », avec « Voir » (filtre le
  tableau sur ce MCP) et « J'ai vu » (`acknowledge_mcp_update`).
- Les outils nouveaux portent une marque « nouveau » et leur politique (confirmation ou interdit).
- Réalisation : un bandeau par instance qui a une mise à jour en attente (`InstanceView.update`), noir et blanc :
  rien n'attend avec une échéance, les outils nouveaux étant soumis à leur politique. Le texte compte aussi les
  outils modifiés et retirés (« 1 nouvel outil et 1 outil retiré »). « Voir » vide la recherche et le filtre
  d'environnement, ne garde que cette ligne (pastille « Filtré sur Kibana Logs (PROD) », qu'on retire) et la
  déplie ; les outils nouveaux puis modifiés viennent en tête, marqués, les outils retirés sont nommés au-dessus de
  la liste. « J'ai vu » appelle `acknowledge_mcp_update(environment, mcp)`.

### Recherche plein texte

- **Insensible aux accents et à la casse** (normalisation NFD puis retrait des diacritiques).
- Porte sur : nom du MCP, environnement, session, politique, état, **et noms et descriptions des outils**.
- Quand la correspondance vient d'un outil, la ligne du MCP se déplie et **montre les outils qui correspondent**,
  le terme surligné.
- Plusieurs mots : tous doivent correspondre.
- Réalisation : `ui/src/lib/search.ts` (`normalize`, `terms`, `highlight`), fonctions pures réutilisables par
  l'onglet Journal. Champs d'une ligne : libellé et identifiant du MCP, « distant » ou « local », libellé et
  identifiant de l'environnement, libellé de la session, état affiché, politique (« lecture autorisé »,
  « confirmation (notification) »…). Champs d'un outil : nom, nom exposé, description. Un mot trouvé dans les champs
  de la ligne y est satisfait ; **les mots restants doivent tous se trouver dans un même outil**, et la ligne se
  déplie sur ces seuls outils (« 2 outils sur 4 correspondent à la recherche. »), mots surlignés. Le surlignage
  découpe le texte en segments rendus comme texte : aucune donnée n'est interprétée comme HTML.

### Filtre par environnement

- « Tous » ou un seul environnement, chacun avec son nombre de MCP.
- **Indépendant de l'exposition** : on peut filtrer sur un environnement non exposé pour voir ce qu'il contient.
- Réalisation : menu déroulant à boutons radio (« Tous », puis chaque environnement), chaque choix avec son nombre
  de MCP, compté sur toutes les instances quels que soient la recherche et l'exposition.

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
- Réalisation :
  - une ligne par instance (un MCP dans un environnement), dans l'ordre de la config ; Data Table shadcn-vue
    (composant `table` et `@tanstack/vue-table` 8.21.3), tri par clic sur chaque en-tête (`aria-sort`) ;
  - Session : point d'état de l'onglet Connexions et libellé ; Politique : une marque par catégorie (L, É, D) et
    une icône de décision, noir et blanc, l'explication du moteur (`Resolution::describe`) en infobulle ; État :
    « injoignable » en rouge (erreur), sa dernière erreur en infobulle et dans la ligne dépliée ;
  - ligne dépliable par son nom : nom, nom exposé, description, catégorie, politique de chaque outil (explication en
    infobulle) et marques « nouveau », « modifié », « inconnu » ; « Recharger la liste » (`reload_mcp`) pour une
    instance exposée ;
  - un MCP coupé l'est dans tous les environnements (`disabled_mcp`, spec 004) : l'interrupteur de chaque ligne
    reflète l'exposition de l'instance, et son infobulle dit « Couper Kibana Logs retire ses outils dans tous les
    environnements. ». Désactivé sans profil choisi ou quand l'environnement n'est pas exposé, avec la raison en
    infobulle et en description accessible ;
  - états vides : « Aucun environnement n'est exposé… » sous les pastilles (le tableau liste quand même tout),
    « Aucun MCP ne correspond à « … ». » avec « Effacer la recherche ».
- Commandes (réalisation) : câblage mince dans `app/src/interface.rs`, logique dans `mcp_proxy_gateway::interface`,
  déclarées dans `build.rs`, données par la seule capacité `main` :
  `set_environment_exposed(environment, exposed)` (refus sans profil choisi pour exposer ; journal « environnement
  « PROD » exposé par l'utilisateur » ou « retiré » ; scellé), `set_mcp_exposed(mcp, exposed)` (journal « MCP
  « Kibana Logs » coupé par l'utilisateur » ou « rétabli »), `acknowledge_mcp_update(environment, mcp)`,
  `reload_mcp(environment, mcp)` (refusé pour une instance non exposée ou un MCP coupé : aucun trafic vers ce que
  l'utilisateur n'a pas choisi). Identifiants vérifiés contre la config, jamais recopiés dans l'erreur ; demander
  l'état courant ne change ni n'écrit rien ; un échec d'enregistrement laisse le changement appliqué et ajoute
  l'alerte `settingsNotSaved`.
- Instantané : `InstanceView` gagne `session` (session utilisée, `null` sans session) et `policy` (politique par
  défaut, `PolicyEngine::summary` : `read`, `write`, `destructive`, chacune `{ decision, label, explanation }`,
  `decision` parmi `allow`, `confirmElicitation`, `confirmNotification`, `deny`) ; `ToolView` gagne `description`
  (description du serveur, filtrée, vide sans) et `policy` (décision actuelle d'un appel à l'outil, `resolve`).

## Tests

- **EX-017-01** — Normalisation : `développeur` trouve `Developpeur`, `PROD` trouve `prod`, `kibána` trouve `Kibana`.
- **EX-017-02** — Recherche sur chaque champ ; correspondance dans une description d'outil → ligne dépliée, outils filtrés ;
  plusieurs mots.
- **EX-017-03** — Filtre par environnement indépendant de l'exposition ; compteurs exacts.
- **EX-017-04** — Pastilles et interrupteurs appellent les bonnes commandes ; interrupteur désactivé quand l'environnement ne l'est
  pas.
- **EX-017-05** — Bandeau : apparition, « Voir », « J'ai vu ».
- **EX-017-06** — Compteur orange au-delà du seuil.
- **EX-017-07** — Playwright : captures de l'onglet (liste, recherche avec outils dépliés, bandeau), en clair et en sombre.
- **EX-017-10** — Instantané : chaque instance porte sa session et sa politique par défaut par catégorie, chaque
  outil sa description et sa politique avec l'explication du moteur (outil inconnu relevé par le profil, verrou de
  la config).
- **EX-017-11** — Commandes de l'onglet sur l'assemblage réel : exposer un environnement (refusé sans profil),
  couper puis rétablir un MCP (ses seuls outils, dans tous les environnements), recharger (refusé hors exposition
  ou MCP coupé), acquitter ; identifiants inconnus refusés ; journal ; choix scellés gardés au redémarrage.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-017-08** (manuel) — Désactiver un environnement retire ses outils du client LLM (après rechargement de la liste côté client).
- **EX-017-09** (manuel) — Couper un MCP retire ses seuls outils.
- **EX-017-12** (manuel) — L'onglet dans la fenêtre de l'appli, en clair et en sombre : infobulles et menus sans
  erreur de CSP, recherche, instance injoignable, rechargement, compteur orange.

## Hors périmètre

Édition des politiques depuis l'interface.

## Amendements

- **2026-11-22** — Implémentation (I017.1 à I017.12). Ajouts : précisions de réalisation dans chaque section ;
  commande `reload_mcp` (rechargement d'une instance exposée) ; confirmation avant d'exposer un environnement
  sensible ; exposition refusée sans profil choisi ; champs `session`, `policy` et `description` de l'instantané ;
  scénarios simulés `seuil`, `volumineux` (40 lignes, plus de 300 outils), `rien-expose`. Écarts au texte :
  l'interrupteur « Exposé » coupe le MCP dans **tous** les environnements (réglage par MCP de la spec 004), ce que
  son infobulle dit ; l'état « session requise » de la spec s'affiche tel quel (l'instantané dit « connexion
  requise ») ; le bandeau de mise à jour est noir et blanc. Critères ajoutés : `EX-017-10`, `EX-017-11`
  (automatiques), `EX-017-12` (recette). Aucun critère retiré.
