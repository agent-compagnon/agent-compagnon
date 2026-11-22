# Recette manuelle sous Windows

Vérifications qui ne peuvent pas être exécutées dans le conteneur Linux (le code Windows y est seulement compilé).
À dérouler sur un poste Windows avec la chaîne Rust du projet.

## Étape 2a — secrets en mémoire

1. `cargo test -p mcp-proxy-core platform::windows` : les 2 tests passent (aller-retour `CryptProtectMemory` /
   `CryptUnprotectMemory` avec un tampon effectivement modifié par le chiffrement ; tampon non aligné refusé).
2. `cargo test -p mcp-proxy-core secrets` : tous les tests passent. `platform_store_round_trip` y utilise
   `WindowsMemoryProtector` ; les tests du client HTTP passent par SChannel (requêtes `http` en boucle locale).
3. Aucun message « secrets en mémoire NON chiffrés » dans la sortie : le protecteur de développement n'existe pas
   sous Windows.
