// DeepSeek Harness (dsh) plugin entry for the easybooks bundle.
//
// The packaged skill tree lives next to this file. A cordis patch cannot name
// its own package directory, so this plugin resolves it at runtime and mounts
// the official filesystem skill provider on exactly that root. The provider is
// isolated (no project or user roots) and ranks above user-level skill copies,
// so the host agent always reads the skill shipped with this exact release.
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as skillFilesystem from '@deepseek-ai/dsh-skill-filesystem'

export const name = 'easybooks'
export const inject = ['skills']

const packageRoot = dirname(fileURLToPath(import.meta.url))

export function apply(ctx) {
  ctx.plugin(skillFilesystem, {
    providerName: 'easybooks',
    includeDefaultRoots: false,
    customSkillDirs: [join(packageRoot, 'skills')],
  })
}
