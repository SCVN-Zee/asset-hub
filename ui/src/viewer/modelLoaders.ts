import * as THREE from 'three'
import { ColladaLoader } from 'three/examples/jsm/loaders/ColladaLoader.js'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
import { OBJLoader } from 'three/examples/jsm/loaders/OBJLoader.js'

/** The file formats of the Model tab, and which three.js loader reads each. */
export type ModelFormat = 'fbx' | 'obj' | 'gltf' | 'glb' | 'dae'

const FORMATS: ReadonlySet<string> = new Set<ModelFormat>(['fbx', 'obj', 'gltf', 'glb', 'dae'])

/** The format of a model file from its extension (any case); FBX when the extension is none of them. */
export function modelFormat(path: string): ModelFormat {
  const ext = /\.([a-z0-9]+)$/i.exec(path)?.[1].toLowerCase() ?? ''
  return FORMATS.has(ext) ? (ext as ModelFormat) : 'fbx'
}

const text = (data: ArrayBuffer) => new TextDecoder().decode(data)

/**
 * Reads a model that is not an FBX (the FBX is read by FBXLoader in the viewer). The result is one group
 * that holds the model's animation clips in `animations`, like a loaded FBX.
 */
export async function parseModel(format: Exclude<ModelFormat, 'fbx'>, data: ArrayBuffer, manager: THREE.LoadingManager): Promise<THREE.Group> {
  switch (format) {
    case 'obj':
      return new OBJLoader(manager).parse(text(data))
    case 'dae': {
      const collada = new ColladaLoader(manager).parse(text(data), '')
      const group = new THREE.Group()
      group.add(...collada.scene.children)
      // The loader returns its clips next to the scene, but its typings leave the field out.
      group.animations = (collada as { animations?: THREE.AnimationClip[] }).animations ?? []
      return group
    }
    case 'gltf':
    case 'glb': {
      const gltf = await new GLTFLoader(manager).parseAsync(format === 'glb' ? data : text(data), '')
      gltf.scene.animations = gltf.animations
      return gltf.scene
    }
  }
}
