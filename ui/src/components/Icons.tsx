// Stroke icons, decorative unless a parent gives them a label.
type IconProps = { size?: number }

const base = (size: number) => ({
  width: size,
  height: size,
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.8,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
  'aria-hidden': true
})

export const CubeIcon = ({ size = 24 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M12 2.5 21 7.5v9L12 21.5 3 16.5v-9z" />
    <path d="M3 7.5 12 12.5l9-5M12 12.5v9" />
  </svg>
)

export const SearchIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-3.5-3.5" />
  </svg>
)

export const FolderIcon = ({ size = 14 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2h8.5A1.5 1.5 0 0 1 21 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z" />
  </svg>
)

export const ChevronRightIcon = ({ size = 12 }: IconProps) => (
  <svg {...base(size)}>
    <path d="m9 6 6 6-6 6" />
  </svg>
)

export const ChevronLeftIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <path d="m15 6-6 6 6 6" />
  </svg>
)

export const GearIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
  </svg>
)

export const RefreshIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M20 11a8 8 0 0 0-14.9-3M4 4v4h4M4 13a8 8 0 0 0 14.9 3M20 20v-4h-4" />
  </svg>
)

export const SphereIcon = ({ size = 24 }: IconProps) => (
  <svg {...base(size)}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M3.5 12c0 2.2 3.8 4 8.5 4s8.5-1.8 8.5-4" />
    <path d="M8.5 7.5a4 4 0 0 1 3-1.5" />
  </svg>
)

export const AudioIcon = ({ size = 24 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M4 10v4M8 7v10M12 4v16M16 8v8M20 11v2" />
  </svg>
)

export const FontIcon = ({ size = 24 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M4 20 9.5 5h1L16 20M6.2 14h7.6M15 20h5M17.5 9v11" />
  </svg>
)

export const AnimIcon = ({ size = 24 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M3 17c3 0 4-10 7-10s3 10 7 10 3-6 4-6" />
    <circle cx="10" cy="7" r="1.4" />
    <circle cx="17" cy="17" r="1.4" />
  </svg>
)

export const ImageIcon = ({ size = 24 }: IconProps) => (
  <svg {...base(size)}>
    <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
    <circle cx="9" cy="9.5" r="1.6" />
    <path d="m20.5 16-5-5-8.5 8.5" />
  </svg>
)

// Actions and places, one family: same grid, stroke and caps as above. `filled` marks an "on" state.
type ToggleIconProps = IconProps & { filled?: boolean }

/** Export a .unitypackage: a package with an arrow coming out of it, down to the disk. */
export const ExportIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M12 3v10m-4-4 4 4 4-4" />
    <path d="M4 14v4.5A1.5 1.5 0 0 0 5.5 20h13a1.5 1.5 0 0 0 1.5-1.5V14" />
  </svg>
)

/** One file saved as it is: a page with a down arrow. */
export const DownloadIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
    <path d="M14 3v5h5" />
    <path d="M12 11v6m-3-3 3 3 3-3" />
  </svg>
)

/** The chest (the app's export list), as in the logo: a domed lid over a box with a clasp. */
export const ChestIcon = ({ size = 16, filled }: ToggleIconProps) => (
  <svg {...base(size)}>
    <path d="M4 10.5a8 4.5 0 0 1 16 0" fill={filled ? 'currentColor' : 'none'} fillOpacity={filled ? 0.25 : 0} />
    <path d="M4 10.5h16V19a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1z" />
    <path d="M10.5 10.5v3h3v-3M4 14h6.5M13.5 14H20" />
  </svg>
)

export const StarIcon = ({ size = 16, filled }: ToggleIconProps) => (
  <svg {...base(size)}>
    <path d="m12 3.5 2.6 5.3 5.9.9-4.25 4.1 1 5.8L12 16.9l-5.25 2.7 1-5.8L3.5 9.7l5.9-.9z" fill={filled ? 'currentColor' : 'none'} />
  </svg>
)

/** A collection: cards stacked. */
export const CollectionIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <rect x="4" y="8" width="13" height="12" rx="1.5" />
    <path d="M7 5h11.5A1.5 1.5 0 0 1 20 6.5V17" />
  </svg>
)

/** A pinned branch: a push pin. */
export const PinIcon = ({ size = 16, filled }: ToggleIconProps) => (
  <svg {...base(size)}>
    <path d="M9 3h6l-1 6 4 4H6l4-4z" fill={filled ? 'currentColor' : 'none'} />
    <path d="M12 13v8" />
  </svg>
)

/** Viewed lately. */
export const ClockIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 7.5V12l3 2" />
  </svg>
)

/** Feedback: a speech bubble. */
export const MessageIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M5 5.5h14a1.5 1.5 0 0 1 1.5 1.5v8.5a1.5 1.5 0 0 1-1.5 1.5H10l-4.5 3.5V17H5a1.5 1.5 0 0 1-1.5-1.5V7A1.5 1.5 0 0 1 5 5.5z" />
    <path d="M8 10h8M8 13h5" />
  </svg>
)

/** Exported lately: a package with an arrow going out. */
export const ShippedIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M12 14V4m-4 4 4-4 4 4" />
    <path d="M4 14v4.5A1.5 1.5 0 0 0 5.5 20h13a1.5 1.5 0 0 0 1.5-1.5V14" />
  </svg>
)

export const FilterIcon = ({ size = 14 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M4 5h16l-6 7.5V19l-4-2v-4.5z" />
  </svg>
)

export const ChevronDownIcon = ({ size = 12 }: IconProps) => (
  <svg {...base(size)}>
    <path d="m6 9 6 6 6-6" />
  </svg>
)

export const CopyIcon = ({ size = 14 }: IconProps) => (
  <svg {...base(size)}>
    <rect x="8" y="8" width="12" height="12" rx="1.5" />
    <path d="M16 8V5.5A1.5 1.5 0 0 0 14.5 4h-9A1.5 1.5 0 0 0 4 5.5v9A1.5 1.5 0 0 0 5.5 16H8" />
  </svg>
)

/** Opens elsewhere (GitHub in the browser). */
export const ExternalIcon = ({ size = 14 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M14 4h6v6M20 4l-9 9" />
    <path d="M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10" />
  </svg>
)

/** An assethub:// link to share. */
export const LinkIcon = ({ size = 14 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1" />
    <path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1" />
  </svg>
)

export const CheckIcon = ({ size = 14 }: IconProps) => (
  <svg {...base(size)}>
    <path d="m5 12.5 4.5 4.5L19 7.5" />
  </svg>
)

/** Unity: the cube with its three arms, as Unity's own mark draws it — for sending a package straight to the editor. */
export const UnityIcon = ({ size = 16 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M12 2.6 20.4 7.4v5.1M20.4 15v1.6L12 21.4l-4.6-2.6M5.3 17.6l-1.7-1V7.4L8.2 4.8" />
    <path d="M12 12v5.4M12 12l4.7-2.7M12 12 7.3 9.3" />
  </svg>
)
