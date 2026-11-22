// Every text of the main window, in French. Components never hold a text of their own.

/** « DEV, INT et REC », « PROD », « » */
export function list(items: readonly string[]): string {
  if (items.length <= 1) return items.join('')
  return `${items.slice(0, -1).join(', ')} et ${items[items.length - 1] ?? ''}`
}

export const fr = {
  app: {
    name: 'MCP Proxy',
    version: (version: string) => `version ${version}`,
    loading: 'Chargement…',
    unavailable: "MCP Proxy ne répond pas : l'état de la passerelle n'a pas pu être lu.",
    outsideApp:
      "Cette page s'affiche dans la fenêtre de MCP Proxy. Ouverte ailleurs, elle n'a accès à aucune donnée.",
    invalidState: "L'état reçu de MCP Proxy est illisible : rien n'est affiché.",
    update: 'Redémarrer pour mettre à jour',
  },
  tabs: {
    label: 'Sections',
    connections: 'Connexions',
    mcp: 'MCP',
    journal: 'Journal',
  },
  shells: {
    mcp: 'Les serveurs MCP, leurs outils et leur exposition apparaîtront ici.',
    journal: 'Les appels et les événements de la passerelle apparaîtront ici.',
  },
  profile: {
    label: 'Profil',
    trigger: (label: string) => `Profil : ${label}`,
    environments: (labels: readonly string[]) =>
      labels.length === 0 ? 'Aucun environnement exposé.' : `Environnements exposés : ${list(labels)}.`,
    sensitive: (labels: readonly string[]) => `Dont sensibles : ${list(labels)}.`,
    firstLaunch: {
      title: 'Choisissez votre profil',
      description:
        "Le profil décide des environnements exposés aux clients LLM et des règles appliquées à leurs appels. Vous pourrez en changer à tout moment, en haut de la fenêtre.",
      choices: 'Profils',
      confirm: 'Continuer',
    },
    change: {
      title: (label: string) => `Passer au profil « ${label} » ?`,
      exposed: (labels: readonly string[]) =>
        labels.length === 0
          ? 'Plus aucun environnement ne sera exposé aux clients LLM.'
          : `Les environnements exposés aux clients LLM reviennent aux valeurs par défaut de ce profil : ${list(labels)}.`,
      sensitive: (labels: readonly string[]) => `Environnements sensibles : ${list(labels)}.`,
      kept: 'Vos durcissements sont conservés.',
      cancel: 'Annuler',
      confirm: 'Changer de profil',
    },
  },
  banners: {
    suspended: "La passerelle est suspendue : tous les appels d'outils sont refusés.",
    resume: 'Reprendre',
    confirmations: (count: number) =>
      count === 1
        ? 'Une confirmation attend votre réponse.'
        : `${String(count)} confirmations attendent votre réponse.`,
    see: 'Voir',
    loginWaiting: (client: string, session: string) => `${client} attend une connexion à ${session}.`,
    unknownClient: 'Un client LLM',
    connect: 'Se connecter',
    commandFailed: (reason: string) => `Action impossible : ${reason}.`,
    dismiss: 'Fermer',
  },
  connections: {
    global: 'SSO globaux',
    perEnvironment: 'SSO par environnement',
    noEnvironment: "Aucun environnement n'est exposé aux clients LLM.",
    state: {
      neverConnected: 'jamais connectée',
      active: (until: string) => `active jusqu'à ${until}`,
      activeTomorrow: (until: string) => `active jusqu'à demain ${until}`,
      expired: 'expirée',
      loginRequested: 'connexion demandée',
    },
    services: 'Services',
    noService: 'Aucun service exposé',
    serviceIn: (mcp: string, environments: readonly string[]) => `${mcp} en ${environments.join(', ')}`,
    connect: 'Se connecter',
    disconnect: 'Déconnecter',
    connectTo: (session: string) => `Se connecter à ${session}`,
    disconnectFrom: (session: string) => `Déconnecter ${session}`,
  },
  basic: {
    attempt: (attempt: number, max: number) => `Essai ${String(attempt)} sur ${String(max)}`,
    login: 'Identifiant',
    password: 'Mot de passe',
    cancel: 'Annuler',
    submit: 'Se connecter',
  },
  confirm: {
    title: 'Confirmation demandée',
    tool: 'Outil',
    client: 'Demandé par',
    policy: 'Politique',
    arguments: 'Arguments',
    expired: "Délai dépassé : l'appel est refusé.",
    remaining: (left: string) => `Sans réponse, l'appel sera refusé dans ${left}.`,
    refuse: 'Refuser',
    approve: 'Autoriser une fois',
  },
} as const
