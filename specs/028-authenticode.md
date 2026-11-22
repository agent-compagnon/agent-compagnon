---
id: "028"
titre: "Signature Authenticode dans la CI"
statut: "backlog"
depend_de: []
suivi: "github#29"
---

# 028 — Signature Authenticode dans la CI

> **Prévu, à ne pas implémenter pour l'instant.**

## Pourquoi

Sans signature Authenticode, Windows peut afficher un avertissement SmartScreen au premier lancement, et certaines
règles d'entreprise (AppLocker, WDAC) peuvent bloquer l'exe. La signature ed25519 de l'updater ne répond pas à ce
besoin : elle protège les mises à jour, pas la confiance de Windows.

## Ce qui est déjà prévu

- Un job `authenticode` désactivé dans `.gitlab-ci.yml` (7c), placé entre `build` et `sign`.

## À faire le moment venu

- Obtenir un certificat de signature de code (autorité interne ou publique) et décider où vit la clé : HSM,
  service de signature, ou magasin de certificats d'un runner dédié.
- Activer le job : `signtool sign /fd SHA256 /tr <horodatage> /td SHA256 mcp-proxy.exe`, puis `signtool verify`.
- **Ordre** : Authenticode modifie l'exe. Il doit donc passer **avant** le calcul du `sha256` et la signature
  ed25519.
- L'updater pourrait vérifier en plus la signature Authenticode de l'exe téléchargé (`WinVerifyTrust`), en
  complément de la vérification ed25519, jamais à sa place.
- Mettre à jour `docs/release.md` et la recette d'installation (7a).

## Questions à trancher

- Certificat interne ou public ?
- Horodatage : quel service est joignable depuis les runners ?
