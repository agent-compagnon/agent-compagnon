import { describe, expect, it } from 'vitest'
import { snapshotOf, spyBackend } from '@/test/fixtures'
import { createStore } from './snapshot'

describe('store of the snapshot', () => {
  // Exigence : EX-016-01
  it('holds the snapshot, then every one of snapshot-changed', async () => {
    const first = snapshotOf('sessions')
    const { backend, emit } = spyBackend(first)
    const store = createStore(backend)
    expect(store.state.phase).toBe('loading')
    await store.start()
    expect(backend.onSnapshot).toHaveBeenCalledBefore(backend.getSnapshot)
    expect(store.state.phase).toBe('ready')
    expect(store.state.snapshot).toEqual(first)

    const next = snapshotOf('suspendue')
    emit(next)
    expect(store.state.snapshot?.gatewaySuspended).toBe(true)
    // A snapshot that could not be read hides everything rather than show a stale state.
    emit(null)
    expect(store.state.phase).toBe('invalid')
    expect(store.state.snapshot).toBeNull()
    store.close()
  })

  // Exigence : EX-016-01
  it('never replaces a newer event by an older answer', async () => {
    const { backend, emit } = spyBackend(snapshotOf('sessions'))
    let answer: (value: ReturnType<typeof snapshotOf>) => void = () => undefined
    backend.getSnapshot.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve
        }),
    )
    const store = createStore(backend)
    const started = store.start()
    await Promise.resolve()
    emit(snapshotOf('suspendue'))
    answer(snapshotOf('sessions'))
    await started
    expect(store.state.snapshot?.gatewaySuspended).toBe(true)
  })

  // Exigence : EX-016-01
  it('says when the application gives no snapshot, and why a command was refused', async () => {
    const { backend } = spyBackend(snapshotOf('sessions'))
    backend.getSnapshot.mockRejectedValueOnce('filtre de sortie indisponible : état non transmis')
    const store = createStore(backend)
    await store.start()
    expect(store.state.phase).toBe('unavailable')

    backend.connectSession.mockRejectedValueOnce('session inconnue')
    expect(await store.connect('sso-x')).toBe(false)
    expect(store.state.commandError).toBe('session inconnue')
    expect(await store.disconnect('sso-dev')).toBe(true)
    expect(store.state.commandError).toBeNull()
    expect(backend.disconnectSession).toHaveBeenCalledWith('sso-dev')
    await store.setProfile('support')
    await store.resume()
    await store.showConfirmations()
    expect(backend.setProfile).toHaveBeenCalledWith('support')
    expect(backend.resumeGateway).toHaveBeenCalledOnce()
    expect(backend.showConfirmations).toHaveBeenCalledOnce()
  })
})
