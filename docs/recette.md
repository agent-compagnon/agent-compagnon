# Recette manuelle sous Windows

Vérifications qui ne peuvent pas être exécutées dans le conteneur Linux (le code Windows y est seulement compilé).
À dérouler sur un poste Windows avec la chaîne Rust du projet.

Une section par spec qui a des critères `(manuel)`, une case par critère. Chaque ligne cite l'identifiant du
critère : `cargo xtask trace` vérifie que tous les critères manuels d'une spec implémentée figurent ici.

## 002 — core — store de secrets en mémoire

- [ ] **EX-002-09** — Aucun message « secrets en mémoire NON chiffrés » dans la sortie : le protecteur de
  développement n'existe pas sous Windows.
- [ ] **EX-002-16** — `cargo test -p mcp-proxy-core platform::windows` : les 2 tests passent (aller-retour DPAPI
  utilisateur réel par WinRT ; données protégées différentes des données claires et sans la valeur en clair ;
  données quelconques refusées ou rendues vides).
- [ ] **EX-002-17** — `cargo test -p mcp-proxy-core secrets` : tous les tests passent. `platform_store_round_trip`
  y utilise `DpapiKeyWrapper`, aller-retour d'essai de `SecretStore::for_platform` compris ; les tests du client HTTP
  passent par SChannel (requêtes `http` en boucle locale).

## 025 — Module MCP local d'exemple (`jira-local`)

À dérouler quand l'appli (spec 011) et les fenêtres de saisie Basic (spec 012) existent, avec une config dont
l'instance `jira` pointe vers un Jira réel et dont `sso-tools` a une `probe_url` sur ce Jira.

- [ ] **EX-025-08** — Depuis un client LLM, rechercher des tickets (`rec_jira_search_issues`) : la saisie Basic de
  « SSO Outils » s'ouvre, puis la réponse liste les tickets attendus, sans adresse e-mail ni lien `self`. Récupérer
  un ticket (`prod_jira_get_issue`). Ajouter un commentaire en PROD (`prod_jira_add_comment`) : la confirmation
  s'affiche ; un refus n'ajoute rien dans Jira, une acceptation ajoute le commentaire. Changer le mot de passe dans
  Jira puis relancer une recherche : une nouvelle saisie est demandée, une seule, et la recherche aboutit.

