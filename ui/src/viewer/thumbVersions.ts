/**
 * Bumped when animation cards are drawn differently, so cards drawn before are
 * drawn once more. v2: models named in the clip's file name are tried too;
 * v3: and those its Animator Controllers name; whether a model was found is kept;
 * v4: models with textures not found are drawn (see `MODEL_THUMB_VERSION`).
 * Kept apart from the renderers so the grid can check them without loading three.js.
 */
export const ANIM_THUMB_VERSION = 4

/** Model and prefab cards; v2: textures not found are white, not transparent (such models were drawn invisible). */
export const MODEL_THUMB_VERSION = 2
