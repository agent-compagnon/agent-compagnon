import { type ClassValue, clsx } from 'clsx'
import { twMerge } from 'tailwind-merge'

/** Tailwind classes merged, later ones winning (shadcn-vue). */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs))
}
