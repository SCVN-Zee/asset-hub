import { useId } from 'react'

/**
 * The XP Hero treasure chest: hero-yellow body, red scarf round the seam,
 * leather straps, gold lock and an ice sword as loot. Below 48 px the small
 * highlights are dropped and the outline thickened so it stays readable.
 */
export function LogoMark({ size = 32 }: { size?: number }) {
  const clip = useId()
  const small = size < 48
  return (
    <svg width={size} height={size} viewBox="0 0 128 128" aria-hidden="true">
      <defs>
        <clipPath id={clip}>
          <rect x="18" y="62" width="92" height="52" rx="10" />
        </clipPath>
      </defs>
      <g stroke="#141414" strokeWidth={small ? 6 : 5} strokeLinejoin="round" strokeLinecap="round">
        <g transform="translate(97 54) rotate(20)">
          <path d="M-11 0V-40L0-54L11-40V0Z" fill="#9CCBFF" />
          {!small && <path d="M-4-6V-39L0-45V-6Z" fill="#FFFFFF" stroke="none" opacity=".75" />}
          <rect x="-16" y="0" width="32" height="9" rx="3" fill="#F5A12E" />
        </g>
        <rect x="18" y="62" width="92" height="52" rx="10" fill="#FFC21A" />
        <g clipPath={`url(#${clip})`} stroke="none">
          <rect x="0" y="98" width="128" height="30" fill="#E8980C" />
        </g>
        <rect x="18" y="62" width="92" height="52" rx="10" fill="none" />
        <path d="M18 64V50C18 34 36 26 64 26S110 34 110 50V64Z" fill="#FFD23F" />
        {!small && <path d="M30 44C34 36 46 33 58 33" fill="none" stroke="#FFF0A8" strokeWidth="5" />}
        <path d="M34 29V114M94 29V114" strokeWidth="14" />
        <path d="M34 29V114M94 29V114" stroke="#7A3E2E" strokeWidth="7" strokeLinecap="butt" />
        <path d="M104 62L120 84L104 88Z" fill="#C92A22" />
        <rect x="16" y="56" width="96" height="14" rx="4" fill="#E8352B" />
        {!small && <path d="M24 60H104" stroke="#FF6E5A" strokeWidth="3" />}
        <rect x="54" y="54" width="20" height="24" rx="5" fill="#F5A12E" />
        <path d="M64 63V69" strokeWidth="4" />
      </g>
    </svg>
  )
}
