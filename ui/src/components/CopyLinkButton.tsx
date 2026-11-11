import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CheckIcon, LinkIcon } from './Icons'
import { assetLink, type AssetLink } from '@shared/deeplink'

/** Copies an assethub:// link: pasted in chat, it opens the asset in a teammate's Asset Hub. */
export function CopyLinkButton({ link, className = 'btn', iconOnly }: { link: AssetLink; className?: string; iconOnly?: boolean }) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  const url = assetLink(link)
  useEffect(() => setCopied(false), [url])
  return (
    <button
      type="button"
      className={className}
      title={iconOnly ? `${copied ? t('details.copied') : t('link.copy')} · ${t('link.hint')}` : t('link.hint')}
      aria-label={copied ? t('details.copied') : t('link.copy')}
      onClick={() =>
        void navigator.clipboard.writeText(url).then(
          () => setCopied(true),
          () => setCopied(false)
        )
      }
    >
      {iconOnly ? (
        <>
          {copied ? <CheckIcon /> : <LinkIcon />}
          <span>{copied ? t('actions.copied') : t('actions.link')}</span>
        </>
      ) : copied ? (
        t('details.copied')
      ) : (
        t('link.copy')
      )}
    </button>
  )
}
