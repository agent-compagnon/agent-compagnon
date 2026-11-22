# Constitution de MCP Proxy

Ce qui ne se négocie pas, quelle que soit la spec. Si une spec contredit ce document, c'est ce document qui
l'emporte, et la spec est à corriger.

## Invariants de sécurité

- **INV-1** — Aucun secret sur disque. Aucun secret dans les logs, les erreurs, les messages renvoyés au LLM, les
  panics ni les `Debug`.
- **INV-2** — Un secret appartient à une seule session et ne part que vers les origines déclarées pour cette
  session.
- **INV-3** — Le LLM ne reçoit aucune donnée qui n'est pas passée par le filtre de sortie.
- **INV-4** — Un module local ne manipule jamais un secret : il reçoit un client HTTP déjà authentifié.
- **INV-5** — Sans réponse de l'utilisateur, une confirmation est refusée. En cas de doute, le code échoue fermé.
- **INV-6** — L'utilisateur peut durcir une règle, jamais l'assouplir.
- **INV-7** — Aucun remplacement de l'exe sans signature ed25519 valide, version cohérente et strictement
  supérieure à la version courante.
- **INV-8** — Les pages distantes (login) n'ont accès à aucune commande de l'appli.
- **INV-9** — Aucune télémétrie : l'appli ne contacte que les MCP configurés et Artifactory.

## Règles de code

- Pas de `unwrap()`, `expect()` ni `panic!` hors des tests.
- `unsafe` confiné aux modules de plateforme, chaque bloc avec son commentaire `// SAFETY:`.
- TLS : magasin de certificats de Windows, vérification jamais désactivée, aucune redirection suivie avec des
  identifiants.
- `app/` ne contient que du câblage Windows ; toute logique testable vit dans `crates/` et se teste sous Linux.
- Interface et messages destinés à l'utilisateur en français ; code, commentaires et commits en anglais ; specs et
  décisions en français.
- Les messages d'erreur renvoyés au LLM sont en français, explicites, actionnables et sans secret.

## Règles de travail

- Une spec à la fois, dans l'ordre des dépendances. Une étape est finie quand `cargo xtask ci` est vert.
- La spec et le code changent dans le même commit. Les critères d'acceptation sont cités par les tests qui les
  prouvent (voir `README.md`).
- Ce qui ne peut pas être exécuté sous Linux (WebView2, toasts, installation, remplacement de l'exe) est livré avec
  ses vérifications manuelles dans `docs/recette.md`.
- Tout point ambigu : choisir l'option la plus sûre, la noter dans `DECISIONS.md`, continuer.
- Chaque message de commit porte la ligne `Spec: NNN` dans son bloc final, avec les lignes d'attribution et sans
  ligne vide entre elles.
- Le dépôt est la seule mémoire du projet : rien d'important ne vit uniquement dans un ticket ou une conversation.

## Amender ce document

Un changement s'ajoute à la section « Amendements » avec sa date et sa raison, et la décision correspondante
s'ajoute à `DECISIONS.md`. Le texte d'un invariant n'est pas réécrit en silence.

## Amendements

_Aucun pour l'instant._
