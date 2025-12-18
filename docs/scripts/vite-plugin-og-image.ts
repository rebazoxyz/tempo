import { readFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import type { Plugin } from 'vite'

const baseUrl = process.env['VITE_BASE_URL'] ||
  (process.env['VERCEL_URL'] ? `https://${process.env['VERCEL_URL']}` : undefined) ||
  (process.env['NODE_ENV'] !== 'production' ? 'http://localhost:5173' : 'https://docs.tempo.xyz')

/**
 * Finds the MDX file for a given path
 */
function findMdxFile(path: string, pagesDir: string): string | null {
  // Normalize path
  const normalizedPath = path.replace(/^\//, '').replace(/\/$/, '')
  
  // Try direct path with .mdx extension
  if (normalizedPath === '') {
    const indexPath = join(pagesDir, 'index.mdx')
    if (existsSync(indexPath)) return indexPath
  } else {
    const directPath = join(pagesDir, `${normalizedPath}.mdx`)
    if (existsSync(directPath)) return directPath
    
    // Try as index.mdx in a directory
    const indexPath = join(pagesDir, normalizedPath, 'index.mdx')
    if (existsSync(indexPath)) return indexPath
  }
  
  return null
}

/**
 * Vite plugin to inject OG image meta tags into HTML
 */
export function ogImagePlugin(): Plugin {
  return {
    name: 'vite-plugin-og-image',
    enforce: 'pre',
    transformIndexHtml(html: string, ctx: { path?: string; filename?: string; [key: string]: unknown }) {
      // Only process pages (not API routes, etc.)
      if (!ctx.path || ctx.path.startsWith('/api/')) {
        return html
      }

      const pagesDir = join(process.cwd(), 'pages')
      const mdxPath = findMdxFile(ctx.path, pagesDir)

      if (!mdxPath) {
        // No MDX file found for this path, skip
        // This is expected for some routes (like 404 pages)
        return html
      }

      try {
        const content = readFileSync(mdxPath, 'utf-8')
        
        // Extract frontmatter
        const frontmatterMatch = content.match(/^---\n([\s\S]*?)\n---\n/)
        if (!frontmatterMatch) {
          return html
        }

        const frontmatter = frontmatterMatch[1]
        
        // Extract title - handle multi-line titles
        const titleMatch = frontmatter.match(/title:\s*(.+?)(?:\n|$)/s)
        const descMatch = frontmatter.match(/description:\s*(.+?)(?:\n|$)/s)

        let title = titleMatch
          ? titleMatch[1].trim().replace(/^["']|["']$/g, '')
          : 'Documentation ⋅ Tempo'
        
        // Ensure title has "• Tempo" branding
        const tempoSuffix = ' • Tempo'
        if (!title.endsWith(tempoSuffix) && !title.endsWith(' • Tempo') && !title.endsWith(' · Tempo')) {
          title = `${title}${tempoSuffix}`
        }
        
        const description = descMatch
          ? descMatch[1].trim().replace(/^["']|["']$/g, '')
          : 'Documentation for Tempo testnet and protocol specifications'

        // Construct OG image URL
        const logoUrl = `${baseUrl}/lockup-light.svg`
        const ogImageUrl = `https://vocs.dev/api/og?logo=${encodeURIComponent(logoUrl)}&title=${encodeURIComponent(title)}&description=${encodeURIComponent(description)}`

        // Escape HTML entities
        const escapedTitle = title.replace(/"/g, '&quot;').replace(/&/g, '&amp;')
        const escapedDescription = description.replace(/"/g, '&quot;').replace(/&/g, '&amp;')

        // Inject OG meta tags before closing </head>
        // Check if og:image already exists to avoid duplicates
        if (html.includes('property="og:image"')) {
          // Replace existing OG tags
          const ogImageRegex = /<meta\s+property="og:image"[^>]*>/g
          const ogImageWidthRegex = /<meta\s+property="og:image:width"[^>]*>/g
          const ogImageHeightRegex = /<meta\s+property="og:image:height"[^>]*>/g
          const ogTitleRegex = /<meta\s+property="og:title"[^>]*>/g
          const ogDescRegex = /<meta\s+property="og:description"[^>]*>/g
          
          html = html.replace(ogImageRegex, `<meta property="og:image" content="${ogImageUrl}" />`)
          html = html.replace(ogImageWidthRegex, '<meta property="og:image:width" content="1200" />')
          html = html.replace(ogImageHeightRegex, '<meta property="og:image:height" content="630" />')
          html = html.replace(ogTitleRegex, `<meta property="og:title" content="${escapedTitle}" />`)
          html = html.replace(ogDescRegex, `<meta property="og:description" content="${escapedDescription}" />`)
        } else {
          // Add new OG tags
          const ogTags = `
    <meta property="og:image" content="${ogImageUrl}" />
    <meta property="og:image:width" content="1200" />
    <meta property="og:image:height" content="630" />
    <meta property="og:title" content="${escapedTitle}" />
    <meta property="og:description" content="${escapedDescription}" />
`
          html = html.replace('</head>', `${ogTags}</head>`)
        }

        return html
      } catch (error) {
        console.warn(`Failed to inject OG image for ${ctx.path}:`, error)
        return html
      }
    },
  }
}
