// Capture script of the `jwt_header` login windows (spec 012), injected in every frame of the window by
// `initialization_script_for_all_frames`. The application replaces the configuration placeholder of the last line
// by escaped JSON data (`mcp_proxy_auth::webview::capture_script`).
//
// It observes `fetch` and `XMLHttpRequest` without ever changing a request or a response. For a request whose URL
// matches the capture pattern, with the expected header and scheme, it hands the header value and the URL to the
// deposit command, then forgets them. It writes nothing to the console, keeps no token in a global variable, waits
// for no answer and swallows every error of its own. The application checks every deposit again.
;(function (config) {
  'use strict'

  var settings
  try {
    settings = {
      prefix: String(config.prefix),
      wildcard: config.wildcard === true,
      header: String(config.header).toLowerCase(),
      scheme: String(config.scheme).toLowerCase(),
      command: String(config.command),
    }
  } catch (e) {
    return
  }

  function noop() {}

  // Tauri's IPC exists in the main frame only; a same-origin frame reaches it through the top window, a
  // cross-origin frame cannot (the access throws) and never deposits.
  function ownInvoke(target) {
    try {
      var internals = target.__TAURI_INTERNALS__
      if (internals && typeof internals.invoke === 'function') {
        return { internals: internals, invoke: internals.invoke }
      }
    } catch (e) {
      // cross-origin frame
    }
    return null
  }
  var ipc = ownInvoke(window)

  function findIpc() {
    if (ipc) return ipc
    try {
      if (window.top && window.top !== window) return ownInvoke(window.top)
    } catch (e) {
      // cross-origin frame
    }
    return null
  }

  function deposit(url, value) {
    try {
      var target = findIpc()
      if (!target) return
      var answer = target.invoke.call(target.internals, settings.command, { url: url, value: value })
      if (answer && typeof answer.then === 'function') answer.then(noop, noop)
    } catch (e) {
      // the page never sees an error of the deposit
    }
  }

  function absolute(raw) {
    try {
      return new window.URL(String(raw), window.location.href)
    } catch (e) {
      return null
    }
  }

  function urlMatches(url) {
    if (!url || url.username || url.password) return false
    var href = url.href
    return settings.wildcard ? href.indexOf(settings.prefix) === 0 : href === settings.prefix
  }

  // `{scheme} {token}`: scheme without regard to case, one space, a token without space.
  function valueMatches(value) {
    if (typeof value !== 'string') return false
    var space = value.indexOf(' ')
    if (space !== settings.scheme.length) return false
    if (value.slice(0, space).toLowerCase() !== settings.scheme) return false
    var token = value.slice(space + 1)
    return token.length > 0 && token.indexOf(' ') === -1
  }

  // Header values are trimmed of HTTP whitespace, as `Headers` and `setRequestHeader` do.
  function normalize(value) {
    return String(value).replace(/^[\t\n\r ]+|[\t\n\r ]+$/g, '')
  }

  function combine(previous, value) {
    return previous === null ? normalize(value) : previous + ', ' + normalize(value)
  }

  function maybeDeposit(url, value) {
    if (value !== null && urlMatches(url) && valueMatches(value)) deposit(url.href, value)
  }

  // Headers given to `fetch`: a `Headers`, an array of pairs or a plain object. Any other iterable is left alone:
  // reading it could consume it and change the request.
  function headerOf(headers) {
    if (headers === undefined || headers === null) return null
    var HeadersType = window.Headers
    if (typeof HeadersType === 'function' && headers instanceof HeadersType) return headers.get(settings.header)
    var found = null
    var i
    if (Array.isArray(headers)) {
      for (i = 0; i < headers.length; i++) {
        var pair = headers[i]
        if (pair && pair.length === 2 && String(pair[0]).toLowerCase() === settings.header) {
          found = combine(found, pair[1])
        }
      }
      return found
    }
    if (typeof headers === 'object' && typeof headers[Symbol.iterator] !== 'function') {
      var names = Object.keys(headers)
      for (i = 0; i < names.length; i++) {
        if (names[i].toLowerCase() === settings.header) found = combine(found, headers[names[i]])
      }
    }
    return found
  }

  function inspectFetch(input, init) {
    var RequestType = window.Request
    var isRequest = typeof RequestType === 'function' && input instanceof RequestType
    var url = absolute(isRequest ? input.url : input)
    var value
    if (init && init.headers !== undefined) {
      value = headerOf(init.headers)
    } else {
      value = isRequest ? input.headers.get(settings.header) : null
    }
    maybeDeposit(url, value)
  }

  var originalFetch = window.fetch
  if (typeof originalFetch === 'function') {
    window.fetch = function fetch(input, init) {
      try {
        inspectFetch(input, init)
      } catch (e) {
        // never in the way of the page
      }
      return originalFetch.apply(this, arguments)
    }
  }

  var Xhr = window.XMLHttpRequest
  if (typeof Xhr === 'function' && Xhr.prototype) {
    var proto = Xhr.prototype
    var originalOpen = proto.open
    var originalSetRequestHeader = proto.setRequestHeader
    var originalSend = proto.send
    var pending = new WeakMap()

    proto.open = function open(method, url) {
      var result = originalOpen.apply(this, arguments)
      try {
        pending.set(this, { url: absolute(url), value: null })
      } catch (e) {
        // never in the way of the page
      }
      return result
    }

    proto.setRequestHeader = function setRequestHeader(name, value) {
      var result = originalSetRequestHeader.apply(this, arguments)
      try {
        var state = pending.get(this)
        if (state && String(name).toLowerCase() === settings.header) state.value = combine(state.value, value)
      } catch (e) {
        // never in the way of the page
      }
      return result
    }

    proto.send = function send() {
      var state = null
      try {
        state = pending.get(this) || null
        pending.delete(this)
      } catch (e) {
        state = null
      }
      var result = originalSend.apply(this, arguments)
      if (state) {
        try {
          maybeDeposit(state.url, state.value)
        } catch (e) {
          // never in the way of the page
        }
      }
      return result
    }
  }
})(__MCP_PROXY_CAPTURE_CONFIG__)
