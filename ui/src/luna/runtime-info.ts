/** The one Luna runtime version the app bundles. Bump together with `scripts/vendor-luna.mjs` and the bundled folder. */
export const LUNA_VERSION = '7.2.0'

/** URL prefix of the Luna runtime files. Vite serves them in dev and copies them into the build. */
export const LUNA_BASE_URL = `/luna/${LUNA_VERSION}/`

/** URL of the iframe page that runs the runtime. */
export const LUNA_HOST_URL = '/luna-host.html'
