// Type declaration for Vite Plugin (vite is available at runtime via vocs)
type Plugin = {
  name: string
  enforce?: 'pre' | 'post'
  transform?: (code: string, id: string) => { code: string; map: string | null } | null
  [key: string]: unknown
}

import { extractTitle, extractDescription, generateFrontmatter } from './generate-seo-metadata'

/**
 * Vite plugin to auto-generate SEO metadata for MDX files
 */
export function seoMetadataPlugin(): Plugin {
  return {
    name: 'vite-plugin-seo-metadata',
    enforce: 'pre',
    transform(code: string, id: string) {
      // Only process MDX files from pages directory
      if (!id.includes('/pages/') || !id.endsWith('.mdx')) {
        return null
      }

      // Check if frontmatter already exists with title and description
      const frontmatterMatch = code.match(/^---\n([\s\S]*?)\n---\n/)
      const hasTitle = frontmatterMatch?.[1]?.includes('title:') ?? false
      const hasDescription = frontmatterMatch?.[1]?.includes('description:') ?? false

      if (hasTitle && hasDescription) {
        // Already has both, skip
        return null
      }

      // Extract metadata from content
      // Use just the filename for title generation if path is complex
      const fileName = id.split('/').pop() || id
      const title = extractTitle(code, fileName)
      const description = extractDescription(code)

      // Generate new content
      let newCode = code

      if (frontmatterMatch) {
        // Update existing frontmatter
        const existingFrontmatter = frontmatterMatch[1]
        const newFrontmatter = generateFrontmatter(title, description, existingFrontmatter)
        newCode = code.replace(/^---\n[\s\S]*?\n---\n/, newFrontmatter)
      } else {
        // Add new frontmatter
        const newFrontmatter = generateFrontmatter(title, description)
        newCode = newFrontmatter + code
      }

      return {
        code: newCode,
        map: null,
      }
    },
  }
}

