# [Proposé] Pont stdio résilient : survivre au redémarrage de l'appli

> Référence commune : `00-cadrage.md`. Dépend de : étapes 5a et 7b.
>
> **Proposé pendant le cadrage, non soumis au QCM.** À écarter si ce n'est pas souhaité.

## Pourquoi

Chaque client LLM lance `mcp-proxy.exe stdio`, qui relaie vers l'appli. Quand l'appli redémarre (mise à jour,
plantage, « Quitter » puis relance), le pipe se ferme, le pont s'arrête, et le client considère son serveur MCP
comme mort. Claude Desktop ne le relance pas : il faut redémarrer Claude Desktop. Avec des mises à jour
automatiques, cela arriverait régulièrement.

## À faire

- Quand le pipe se ferme, le pont **ne s'arrête pas**. Il attend le retour de l'appli (nouvelles tentatives pendant
  60 s, relance de l'appli si elle n'est pas là), puis se reconnecte.
- **Ancien protocole** (handshake) : le pont garde en mémoire la requête `initialize` et la notification
  `initialized` du client. À la reconnexion, il les rejoue vers la nouvelle instance et absorbe la réponse, que le
  client a déjà reçue.
- **2026-07-28** (sans session) : rien à rejouer.
- Requêtes en cours au moment de la coupure : le pont répond lui-même une erreur claire pour chacune
  (`MCP Proxy redémarre, réessayez dans un instant.`). Il suit donc les identifiants des requêtes sans réponse.
- Après reconnexion, les secrets ont disparu : la connexion à la demande reprend normalement au premier appel.
- Le pont reste sans secret et sans logique métier. Il ne lit des messages que ce qu'il faut pour suivre les
  identifiants et repérer `initialize`.
- Au-delà de 60 s sans appli : le pont s'arrête avec un message sur stderr, comme aujourd'hui.

### Organisation du code

- `gateway::bridge::ResilientRelay`, générique sur les flux et sur une fabrique de connexion, testé sous Linux avec
  des flux en mémoire.

## Tests

- Coupure puis retour : le client ne voit aucune fermeture de flux.
- Ancien protocole : `initialize` rejoué, réponse dupliquée absorbée, appels suivants servis.
- 2026-07-28 : appels servis après reconnexion, sans rejeu.
- Requêtes en cours à la coupure : chacune reçoit exactement une erreur.
- Appli absente plus de 60 s : arrêt propre.
- Messages volumineux et messages fragmentés correctement découpés.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- Client LLM connecté, puis « Quitter » et relance de l'appli : le client continue de fonctionner sans être
  redémarré.
- Même essai avec une mise à jour suivie de « Redémarrer pour mettre à jour ».
