/**
 * Bumped when animation cards are drawn differently, so cards drawn before are
 * drawn once more. v2: models named in the clip's file name are tried too;
 * v3: and those its Animator Controllers name; whether a model was found is kept;
 * v4: models with textures not found are drawn (see `MODEL_THUMB_VERSION`);
 * v5: drawn on a transparent background, so the grid's grey stage shows through;
 * v6: curve cards are square like the grid cell, so their label is no longer cropped;
 * v7: drawn without the floor grid.
 * Kept apart from the renderers so the grid can check them without loading three.js.
 */
export const ANIM_THUMB_VERSION = 7

/**
 * Model, prefab, material and texture cards; v2: textures not found are white, not transparent (such models were drawn invisible);
 * v3: drawn on a transparent background, so the grid's grey stage shows through;
 * v4: drawn without the floor grid, with neutral lighting.
 */
export const MODEL_THUMB_VERSION = 4

/**
 * Audio, font and TextMeshPro font cards; v1: drawn on a transparent background with dark ink
 * (they had a dark fill), so the grid's grey stage shows through. Stored in the same
 * `modelThumb` field as the model kinds; a kind only ever has one of the two versions.
 */
export const FLAT_THUMB_VERSION = 1
