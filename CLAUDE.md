# MCP Proxy — consignes pour les agents

Application Windows en Rust : passerelle sécurisée entre des clients LLM et des serveurs MCP internes. Le LLM ne voit
jamais un secret.

## Où est la vérité

Le dépôt est la seule mémoire du projet. Rien d'important ne doit vivre uniquement dans un ticket ou une
conversation.

- `specs/constitution.md` — invariants de sécurité et règles. À lire avant toute modification.
- `specs/architecture.md` — architecture, stack, faits techniques vérifiés.
- `specs/NNN-*.md` — une spec par fonctionnalité. `specs/README.md` donne l'index, les statuts et les règles
  d'alignement.
- `DECISIONS.md` — journal des décisions, en ajout seul.
- `docs/recette.md` — vérifications manuelles sous Windows.

## Avant de dire qu'un travail est fini

- `cargo xtask ci` est vert : formatage, clippy hôte, tests, clippy en cible Windows, traçabilité, puis `ui/` s'il
  existe.
- La spec concernée est à jour **dans le même commit** : comportement, critères `EX-…`, statut, et une entrée datée
  dans « Amendements » si le comportement s'écarte de ce qui était écrit.
- Chaque critère touché est cité par un test (`// Exigence : EX-003-07`) ou, s'il est `(manuel)`, par
  `docs/recette.md`.
- Toute décision prise en route est ajoutée à `DECISIONS.md`. Une décision n'est jamais réécrite : elle est marquée
  « remplacée par … ».

## Règles de code

- Pas de `unwrap()`, `expect()` ni `panic!` hors des tests.
- Aucun `unsafe` (`forbid` au niveau du workspace) : une API Windows qui n'existe qu'en `unsafe` se remplace par une
  API sûre (WinRT du crate `windows`, tokio, Tauri, `winreg`) ou ne s'utilise pas.
- Aucun secret dans un log, une erreur, un `Debug` ou un message renvoyé au LLM.
- Code, commentaires et commits en anglais. Textes destinés à l'utilisateur, specs et décisions en français.
- `app/` ne contient que du câblage Windows. La logique testable vit dans `crates/`.

## Commits

Messages en anglais. La ligne `Spec: NNN` (la spec servie) fait partie du bloc final du message, avec les lignes
d'attribution et **sans ligne vide entre elles**, pour que git la lise comme un « trailer ».
`git log --grep "Spec: 003"` doit suffire à retrouver l'histoire d'une spec.

## Ce qui ne peut pas être exécuté ici

Le code Windows est contrôlé à la compilation (`--target x86_64-pc-windows-msvc`) et jamais exécuté sous Linux. Tout
ce qui dépend de WebView2, des toasts, de l'installation ou du remplacement de l'exe se vérifie à la main : les
étapes s'ajoutent à `docs/recette.md`.
