// Tests of the capture script of the login windows (spec 012). The script lives in the `auth` crate, which includes
// it with `include_str!`; here it runs against a fake window with fake `fetch` and `XMLHttpRequest`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import source from '../../crates/auth/src/webview/capture.js?raw'

const PLACEHOLDER = '__MCP_PROXY_CAPTURE_CONFIG__'
const PAGE = 'https://portal.example.internal/home'
const API = 'https://api.example.internal/v1/me'
const TOKEN = 'eyJhbGciOiJSUzI1NiJ9.eyJleHAiOjF9.c2lnbmF0dXJl'
const BEARER = `Bearer ${TOKEN}`

interface CaptureConfig {
  prefix: string
  wildcard: boolean
  header: string
  scheme: string
  command: string
}

const CONFIG: CaptureConfig = {
  prefix: 'https://api.example.internal/',
  wildcard: true,
  header: 'authorization',
  scheme: 'Bearer',
  command: 'deposit_login_header',
}

interface Deposit {
  cmd: string
  args: unknown
}

type Invoke = (cmd: string, args: unknown) => unknown

class FakeXhr {
  static calls: { method: string; args: unknown[]; self: FakeXhr }[] = []
  open(...args: unknown[]): string {
    FakeXhr.calls.push({ method: 'open', args, self: this })
    return 'open-result'
  }
  setRequestHeader(...args: unknown[]): string {
    FakeXhr.calls.push({ method: 'setRequestHeader', args, self: this })
    return 'header-result'
  }
  send(...args: unknown[]): string {
    FakeXhr.calls.push({ method: 'send', args, self: this })
    return 'send-result'
  }
}

interface FakeWindow {
  location: { href: string }
  URL: typeof URL
  Request: typeof Request
  Headers: typeof Headers
  fetch: (...args: unknown[]) => unknown
  XMLHttpRequest: typeof FakeXhr
  __TAURI_INTERNALS__?: { invoke: Invoke }
  top?: unknown
}

interface Harness {
  win: FakeWindow
  deposits: Deposit[]
  fetchCalls: { args: unknown[]; self: unknown }[]
  response: Promise<string>
}

function run(win: FakeWindow, config: CaptureConfig = CONFIG): void {
  const script = source.replace(PLACEHOLDER, JSON.stringify(config))
  // The script is meant to run as page code: evaluated here with the fake window as its `window`.
  // eslint-disable-next-line @typescript-eslint/no-implied-eval
  const evaluate = new Function('window', script) as (w: FakeWindow) => void
  evaluate(win)
}

function harness(options: { invoke?: Invoke; config?: CaptureConfig; internals?: boolean } = {}): Harness {
  const deposits: Deposit[] = []
  const fetchCalls: { args: unknown[]; self: unknown }[] = []
  const response = Promise.resolve('response')
  const invoke: Invoke =
    options.invoke ??
    ((cmd, args) => {
      deposits.push({ cmd, args })
      return Promise.resolve(null)
    })
  const win: FakeWindow = {
    location: { href: PAGE },
    URL,
    Request,
    Headers,
    fetch: function (this: unknown, ...args: unknown[]) {
      fetchCalls.push({ args, self: this })
      return response
    },
    XMLHttpRequest: FakeXhr,
  }
  if (options.internals !== false) win.__TAURI_INTERNALS__ = { invoke }
  win.top = win
  run(win, options.config)
  return { win, deposits, fetchCalls, response }
}

function xhr(win: FakeWindow, url: string, headers: [string, string][]): void {
  const request = new win.XMLHttpRequest()
  request.open('GET', url)
  for (const [name, value] of headers) request.setRequestHeader(name, value)
  request.send()
}

const consoleMethods = ['log', 'info', 'warn', 'error', 'debug', 'trace'] as const
let spies: ReturnType<typeof vi.spyOn>[] = []

beforeEach(() => {
  FakeXhr.calls = []
  // A fresh prototype for every test: the script patches it.
  FakeXhr.prototype.open = function (this: FakeXhr, ...args: unknown[]) {
    FakeXhr.calls.push({ method: 'open', args, self: this })
    return 'open-result'
  }
  FakeXhr.prototype.setRequestHeader = function (this: FakeXhr, ...args: unknown[]) {
    FakeXhr.calls.push({ method: 'setRequestHeader', args, self: this })
    return 'header-result'
  }
  FakeXhr.prototype.send = function (this: FakeXhr, ...args: unknown[]) {
    FakeXhr.calls.push({ method: 'send', args, self: this })
    return 'send-result'
  }
  spies = consoleMethods.map((method) => vi.spyOn(console, method))
})

afterEach(() => {
  for (const spy of spies) expect(spy).not.toHaveBeenCalled()
  vi.restoreAllMocks()
})

const expected = (url: string = API): Deposit[] => [
  { cmd: 'deposit_login_header', args: { url, value: BEARER } },
]

describe('capture script', () => {
  // Exigence : EX-012-08
  it('deposits once for a matching fetch, whatever the form of its headers', () => {
    const forms: [string, (h: Harness) => unknown][] = [
      ['object', (h) => h.win.fetch(API, { headers: { Authorization: BEARER } })],
      ['object, other case', (h) => h.win.fetch(API, { method: 'POST', headers: { aUtHoRiZaTiOn: BEARER } })],
      ['array', (h) => h.win.fetch(API, { headers: [['authorization', BEARER], ['accept', 'json']] })],
      ['Headers', (h) => h.win.fetch(API, { headers: new Headers({ Authorization: BEARER }) })],
      ['Request', (h) => h.win.fetch(new Request(API, { headers: { Authorization: BEARER } }))],
      ['Request and init', (h) => h.win.fetch(new Request(API), { headers: { Authorization: BEARER } })],
      ['URL object', (h) => h.win.fetch(new URL(API), { headers: { Authorization: BEARER } })],
      ['lower-case scheme', (h) => h.win.fetch(API, { headers: { Authorization: BEARER.replace('Bearer', 'bearer') } })],
      ['padded value', (h) => h.win.fetch(API, { headers: { Authorization: `  ${BEARER}\t` } })],
    ]
    for (const [name, send] of forms) {
      const h = harness()
      send(h)
      const value = name === 'lower-case scheme' ? BEARER.replace('Bearer', 'bearer') : BEARER
      expect(h.deposits, name).toEqual([{ cmd: 'deposit_login_header', args: { url: API, value } }])
      expect(h.fetchCalls, name).toHaveLength(1)
    }
  })

  // Exigence : EX-012-08
  it('resolves relative URLs against the page', () => {
    const h = harness()
    h.win.location.href = 'https://api.example.internal/app/'
    h.win.fetch('../v1/me', { headers: { Authorization: BEARER } })
    expect(h.deposits).toEqual(expected())
  })

  // Exigence : EX-012-08
  it('deposits once for a matching XMLHttpRequest', () => {
    const h = harness()
    xhr(h.win, API, [
      ['Accept', 'application/json'],
      ['Authorization', BEARER],
    ])
    expect(h.deposits).toEqual(expected())
    // The request is forgotten once sent: sending it again deposits nothing more.
    const again = FakeXhr.calls.find((c) => c.method === 'send')?.self
    again?.send()
    expect(h.deposits).toHaveLength(1)
  })

  // Exigence : EX-012-08
  it('deposits nothing for a request that does not match', () => {
    const cases: [string, string, Record<string, string>][] = [
      ['neighbour host', 'https://api.example.internal.evil/v1', { Authorization: BEARER }],
      ['subdomain', 'https://x.api.example.internal/v1', { Authorization: BEARER }],
      ['http', 'http://api.example.internal/v1', { Authorization: BEARER }],
      ['identifiers', 'https://user:pw@api.example.internal/v1', { Authorization: BEARER }],
      ['other header', API, { 'X-Authorization': BEARER }],
      ['no header', API, {}],
      ['other scheme', API, { Authorization: `Basic ${TOKEN}` }],
      ['no scheme', API, { Authorization: TOKEN }],
      ['two spaces', API, { Authorization: `Bearer  ${TOKEN}` }],
      ['empty token', API, { Authorization: 'Bearer ' }],
      ['token with space', API, { Authorization: `Bearer ${TOKEN} x` }],
      ['longer scheme', API, { Authorization: `Bearerx ${TOKEN}` }],
    ]
    for (const [name, url, headers] of cases) {
      const h = harness()
      h.win.fetch(url, { headers })
      xhr(h.win, url, Object.entries(headers))
      expect(h.deposits, name).toEqual([])
      expect(h.fetchCalls, name).toHaveLength(1)
    }
  })

  // Exigence : EX-012-08
  it('follows an exact pattern when there is no wildcard', () => {
    const h = harness({ config: { ...CONFIG, prefix: API, wildcard: false } })
    h.win.fetch(`${API}/more`, { headers: { Authorization: BEARER } })
    h.win.fetch(`${API}?q=1`, { headers: { Authorization: BEARER } })
    expect(h.deposits).toEqual([])
    h.win.fetch(API, { headers: { Authorization: BEARER } })
    expect(h.deposits).toEqual(expected())
  })

  // Exigence : EX-012-08
  it('passes requests and responses through unchanged', () => {
    const h = harness()
    const init = { method: 'POST', headers: { Authorization: BEARER }, body: 'payload' }
    const extra = { marker: true }
    const result = h.win.fetch.call(h.win, API, init, extra)
    expect(result).toBe(h.response)
    expect(h.fetchCalls).toHaveLength(1)
    const call = h.fetchCalls[0]
    expect(call?.self).toBe(h.win)
    expect(call?.args).toHaveLength(3)
    expect(call?.args[0]).toBe(API)
    expect(call?.args[1]).toBe(init)
    expect(call?.args[2]).toBe(extra)
    expect(init).toEqual({ method: 'POST', headers: { Authorization: BEARER }, body: 'payload' })

    const request = new Request(API, { method: 'PUT', headers: { Authorization: BEARER }, body: 'body' })
    h.win.fetch(request)
    expect(h.fetchCalls[1]?.args[0]).toBe(request)
    expect(request.bodyUsed).toBe(false)

    // A one-shot iterable of headers is not read: reading it would change the request.
    let consumed = false
    const iterable = {
      *[Symbol.iterator]() {
        consumed = true
        yield ['Authorization', BEARER]
      },
    }
    h.win.fetch(API, { headers: iterable })
    expect(consumed).toBe(false)

    const target = new h.win.XMLHttpRequest()
    expect(target.open('GET', API, true)).toBe('open-result')
    expect(target.setRequestHeader('Authorization', BEARER)).toBe('header-result')
    expect(target.send('data')).toBe('send-result')
    expect(FakeXhr.calls.map((c) => [c.method, c.args, c.self === target])).toEqual([
      ['open', ['GET', API, true], true],
      ['setRequestHeader', ['Authorization', BEARER], true],
      ['send', ['data'], true],
    ])
  })

  // Exigence : EX-012-08
  it('keeps every error of the deposit away from the page', async () => {
    const throwing = harness({
      invoke: () => {
        throw new Error('refused')
      },
    })
    expect(throwing.win.fetch(API, { headers: { Authorization: BEARER } })).toBe(throwing.response)
    expect(() => {
      xhr(throwing.win, API, [['Authorization', BEARER]])
    }).not.toThrow()

    const rejecting = harness({ invoke: () => Promise.reject(new Error('refused')) })
    expect(rejecting.win.fetch(API, { headers: { Authorization: BEARER } })).toBe(rejecting.response)
    xhr(rejecting.win, API, [['Authorization', BEARER]])
    // Let the rejections settle: an unhandled one would fail the test run.
    await new Promise((resolve) => setTimeout(resolve, 0))

    const without = harness({ internals: false })
    expect(without.win.fetch(API, { headers: { Authorization: BEARER } })).toBe(without.response)
    expect(without.deposits).toEqual([])
  })

  // Exigence : EX-012-08
  it('keeps nothing in a global variable', () => {
    const globalsBefore = Object.getOwnPropertyNames(globalThis).sort()
    const h = harness()
    const keysBefore = Object.keys(h.win).sort()
    h.win.fetch(API, { headers: { Authorization: BEARER } })
    xhr(h.win, API, [['Authorization', BEARER]])
    expect(h.deposits).toHaveLength(2)
    expect(Object.keys(h.win).sort()).toEqual(keysBefore)
    expect(Object.getOwnPropertyNames(globalThis).sort()).toEqual(globalsBefore)
    for (const value of Object.values(h.win)) {
      expect(typeof value === 'string' && value.includes(TOKEN)).toBe(false)
    }
    const prototypeNames = Object.getOwnPropertyNames(FakeXhr.prototype).sort()
    expect(prototypeNames).toEqual(['constructor', 'open', 'send', 'setRequestHeader'])
  })

  // Exigence : EX-012-08
  it('deposits from a same-origin frame through the top window, never from a cross-origin one', () => {
    const top = harness()
    const frame: FakeWindow = {
      location: { href: PAGE },
      URL,
      Request,
      Headers,
      fetch: () => 'frame-response',
      XMLHttpRequest: FakeXhr,
      top: top.win,
    }
    run(frame)
    frame.fetch(API, { headers: { Authorization: BEARER } })
    expect(top.deposits).toEqual(expected())

    const foreign: FakeWindow = {
      location: { href: 'https://other.example/' },
      URL,
      Request,
      Headers,
      fetch: () => 'foreign-response',
      XMLHttpRequest: FakeXhr,
    }
    Object.defineProperty(foreign, 'top', {
      get() {
        throw new DOMException('Blocked a frame', 'SecurityError')
      },
    })
    run(foreign)
    expect(foreign.fetch(API, { headers: { Authorization: BEARER } })).toBe('foreign-response')
    expect(top.deposits).toHaveLength(1)
  })
})
