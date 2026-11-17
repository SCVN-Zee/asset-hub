import { describe, expect, it } from 'vitest'
import { modelFormat } from './modelLoaders'

describe('modelFormat', () => {
  it('picks the format from the extension, whatever its case', () => {
    expect(modelFormat('A/Hero.FBX')).toBe('fbx')
    expect(modelFormat('A/hero.obj')).toBe('obj')
    expect(modelFormat('A/hero.GLB')).toBe('glb')
    expect(modelFormat('A/hero.gltf')).toBe('gltf')
    expect(modelFormat('A/hero.Dae')).toBe('dae')
  })

  it('falls back to FBX for anything else', () => {
    expect(modelFormat('A/hero')).toBe('fbx')
    expect(modelFormat('A/hero.png')).toBe('fbx')
  })
})
