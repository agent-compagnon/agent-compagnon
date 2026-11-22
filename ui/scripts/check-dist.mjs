// Exigence : EX-016-08
// The application's build must not contain the simulated backend (spec 016) nor the simulated installation (spec
// 019): their markers, their scenarios and the example snapshot are searched for in every file of `dist/`.
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

const MARKERS = [
  'mcp-proxy-simulated-backend',
  'premier-lancement',
  'reglages-reinitialises',
  'Claude Desktop',
  'mcp-proxy-simulated-installer',
  'mise-a-jour-echec',
]

function files(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    return statSync(path).isDirectory() ? files(path) : [path]
  })
}

const dist = process.argv[2] ?? 'dist'
const found = files(dist).flatMap((path) => {
  const content = readFileSync(path, 'utf8')
  return MARKERS.filter((marker) => content.includes(marker)).map((marker) => `${path}: ${marker}`)
})
if (found.length > 0) {
  console.error(`Le backend simulé est dans la construction de l'appli :\n${found.join('\n')}`)
  process.exit(1)
}
console.log(`${dist} : aucun reste du backend simulé`)
