import { describe, expect, it } from 'vitest'
import css from './style.css?raw'

type Rgb = [number, number, number]

/** `oklch(L C H)` to linear sRGB (Björn Ottosson's matrices), clamped to the gamut. */
function oklchToLinear(value: string): Rgb {
  const match = /oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*\)/.exec(value)
  if (!match) throw new Error(`not an opaque oklch color: ${value}`)
  const [l, c, h] = [Number(match[1]), Number(match[2]), (Number(match[3]) * Math.PI) / 180]
  const a = c * Math.cos(h)
  const b = c * Math.sin(h)
  const l_ = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3
  const m_ = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3
  const s_ = (l - 0.0894841775 * a - 1.291485548 * b) ** 3
  const clamp = (x: number) => Math.min(1, Math.max(0, x))
  return [
    clamp(4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_),
    clamp(-1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_),
    clamp(-0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_),
  ]
}

function luminance(value: string): number {
  const [r, g, b] = oklchToLinear(value)
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number]
  return (hi + 0.05) / (lo + 0.05)
}

/** The tokens of `:root` (light) and of the dark media query. */
function tokens(block: 'light' | 'dark'): Record<string, string> {
  const darkStart = css.indexOf('@media (prefers-color-scheme: dark) {')
  const source = block === 'light' ? css.slice(css.indexOf(':root {'), darkStart) : css.slice(darkStart)
  const end = source.indexOf('}')
  const out: Record<string, string> = {}
  for (const [, name, value] of source.slice(0, end).matchAll(/--([\w-]+):\s*([^;]+);/g)) {
    if (name && value) out[name] = value.trim()
  }
  return out
}

// Text on its background: 4.5:1 (WCAG AA); the orange dot against the page: 3:1 (non-text).
const TEXT_PAIRS = [
  ['foreground', 'background'],
  ['card-foreground', 'card'],
  ['muted-foreground', 'background'],
  ['muted-foreground', 'card'],
  ['muted-foreground', 'muted'],
  ['primary-foreground', 'primary'],
  ['attention-text', 'background'],
  ['attention-text', 'card'],
  ['attention-text', 'attention-muted'],
  ['attention-foreground', 'attention'],
  ['destructive-text', 'background'],
  ['destructive-text', 'destructive-muted'],
] as const

describe('theme', () => {
  for (const scheme of ['light', 'dark'] as const) {
    // Exigence : EX-016-03
    it(`has readable contrasts in ${scheme}`, () => {
      const t = tokens(scheme)
      for (const [text, background] of TEXT_PAIRS) {
        const ratio = contrast(t[text] ?? '', t[background] ?? '')
        expect(ratio, `${text} on ${background}: ${ratio.toFixed(2)}`).toBeGreaterThanOrEqual(4.5)
      }
      expect(contrast(t.attention ?? '', t.background ?? '')).toBeGreaterThanOrEqual(3)
    })
  }

  // Exigence : EX-016-03
  it('keeps orange and red for attention and errors only', () => {
    const t = tokens('light')
    const chroma = (value: string) => Number(/oklch\(\s*[\d.]+\s+([\d.]+)/.exec(value)?.[1] ?? 'NaN')
    const colored = Object.entries(t).filter(([, value]) => value.startsWith('oklch') && chroma(value) > 0.01)
    expect(colored.map(([name]) => name).sort()).toEqual(
      [
        'attention',
        'attention-muted',
        'attention-text',
        'destructive',
        'destructive-muted',
        'destructive-text',
      ].sort(),
    )
  })
})
