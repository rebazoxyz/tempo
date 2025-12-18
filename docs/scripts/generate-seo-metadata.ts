import { readFileSync, writeFileSync, existsSync, readdirSync } from 'node:fs'
import { join, relative } from 'node:path'

/**
 * Truncates description to exactly 160 characters max, cutting at word boundary when possible
 */
function truncateDescription(description: string, maxLength: number = 160): string {
  if (description.length <= maxLength) {
    return description
  }

  // Try to cut at word boundary
  const truncated = description.substring(0, maxLength)
  const lastSpace = truncated.lastIndexOf(' ')
  
  // If we can cut at a word boundary within 10 chars of max, do it
  if (lastSpace > maxLength - 10) {
    return truncated.substring(0, lastSpace).trim()
  }
  
  // Otherwise just cut at maxLength
  return truncated.trim()
}

/**
 * Appends "• Tempo" to title if not already present
 */
function appendTempoBranding(title: string): string {
  const tempoSuffix = ' • Tempo'
  // Check if title already ends with "• Tempo" (with or without the bullet point character)
  if (title.endsWith(tempoSuffix) || title.endsWith(' • Tempo') || title.endsWith(' · Tempo')) {
    return title
  }
  return `${title}${tempoSuffix}`
}

/**
 * Extracts title from H1 heading or generates from filename
 */
export function extractTitle(content: string, filePath: string): string {
  // Try to find H1 heading
  const h1Match = content.match(/^#\s+(.+)$/m)
  if (h1Match && h1Match[1]) {
    let title = h1Match[1].trim()
    // Remove markdown formatting (backticks, bold, etc.)
    title = title.replace(/`([^`]+)`/g, '$1')
    title = title.replace(/\*\*([^*]+)\*\*/g, '$1')
    title = title.replace(/\*([^*]+)\*/g, '$1')
    // Remove brackets like [Documentation, integration guides, and protocol specifications]
    title = title.replace(/\s*\[.*?\]/g, '')
    // Capitalize first letter
    if (title.length > 0) {
      title = title.charAt(0).toUpperCase() + title.slice(1)
    }
    return appendTempoBranding(title)
  }

  // Fallback: generate from filename
  const fileName = filePath.split('/').pop()?.replace(/\.mdx?$/, '') || ''
  const title = fileName
    .split(/[-_]/)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ')
  return appendTempoBranding(title)
}

/**
 * Extracts description from first paragraph or intro text
 */
export function extractDescription(content: string): string {
  // Remove frontmatter if present
  let body = content.replace(/^---\n[\s\S]*?\n---\n/, '')
  
  // Remove imports
  body = body.replace(/^import\s+.*?from\s+['"].*?['"];?\n/gm, '')
  
  // Remove H1 heading
  body = body.replace(/^#\s+.+$/m, '')
  
  // Find first paragraph (non-empty line after H1, skipping code blocks)
  const lines = body.split('\n')
  let inCodeBlock = false
  let paragraphLines: string[] = []
  
  for (const line of lines) {
    // Track code block state
    if (line.trim().startsWith('```')) {
      inCodeBlock = !inCodeBlock
      continue
    }
    
    if (inCodeBlock) continue
    
    // Skip empty lines, headings, and other markdown elements
    const trimmed = line.trim()
    if (
      trimmed === '' ||
      trimmed.startsWith('#') ||
      trimmed.startsWith('import ') ||
      trimmed.startsWith(':::') ||
      trimmed.startsWith('<')
    ) {
      if (paragraphLines.length > 0) break
      continue
    }
    
    paragraphLines.push(line.trim())
    
    // Stop after first substantial paragraph (3+ sentences or 150+ chars)
    const text = paragraphLines.join(' ')
    if (text.length > 150 || (text.match(/[.!?]\s/g)?.length ?? 0) >= 2) {
      break
    }
  }
  
  let description = paragraphLines.join(' ').trim()
  
  // Clean up markdown formatting
  description = description
    .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1') // Remove links, keep text
    .replace(/`([^`]+)`/g, '$1') // Remove code backticks
    .replace(/\*\*([^*]+)\*\*/g, '$1') // Remove bold
    .replace(/\*([^*]+)\*/g, '$1') // Remove italic
    .replace(/\n+/g, ' ') // Replace newlines with spaces
    .replace(/\s+/g, ' ') // Normalize whitespace
  
  // Truncate to max 160 characters
  description = truncateDescription(description)
  
  return description || 'Documentation for Tempo testnet and protocol specifications'
}

/**
 * Generates frontmatter YAML
 */
export function generateFrontmatter(title: string, description: string, existingFrontmatter?: string): string {
  const frontmatter: Record<string, string | boolean> = {}
  
  // Parse existing frontmatter if present
  if (existingFrontmatter) {
    const lines = existingFrontmatter.split('\n')
    for (const line of lines) {
      const match = line.match(/^(\w+):\s*(.+)$/)
      if (match && match[1] && match[2]) {
        const key = match[1]
        let value: string | boolean = match[2].trim()
        if (value === 'true') value = true
        if (value === 'false') value = false
        frontmatter[key] = value
      }
    }
  }
  
  // Add or update title and description
  // Ensure title has "• Tempo" branding
  frontmatter['title'] = appendTempoBranding(title)
  frontmatter['description'] = description
  
  // Build YAML string
  const yamlLines = Object.entries(frontmatter).map(([key, value]) => {
    if (typeof value === 'boolean') {
      return `${key}: ${value}`
    }
    return `${key}: ${JSON.stringify(value)}`
  })
  
  return `---\n${yamlLines.join('\n')}\n---\n\n`
}

/**
 * Processes a single MDX file
 */
function processFile(
  filePath: string,
  dryRun: boolean = false,
): { updated: boolean; title: string; description: string } {
  const content = readFileSync(filePath, 'utf-8')

  // Check if frontmatter already exists with title and description
  const frontmatterMatch = content.match(/^---\n([\s\S]*?)\n---\n/)
  const hasTitle = frontmatterMatch?.[1]?.includes('title:')
  const hasDescription = frontmatterMatch?.[1]?.includes('description:')

  // Skip if both already exist
  if (hasTitle && hasDescription && !dryRun) {
    // Already has both, skip
    return { updated: false, title: '', description: '' }
  }

  // Extract metadata
  const title = extractTitle(content, filePath)
  const description = truncateDescription(extractDescription(content))

  if (dryRun) {
    return { updated: true, title, description }
  }

  // Generate new content
  let newContent = content

  if (frontmatterMatch) {
    // Update existing frontmatter
    const existingFrontmatter = frontmatterMatch[1]
    const newFrontmatter = generateFrontmatter(title, description, existingFrontmatter)
    newContent = content.replace(/^---\n[\s\S]*?\n---\n/, newFrontmatter)
  } else {
    // Add new frontmatter
    const newFrontmatter = generateFrontmatter(title, description)
    newContent = newFrontmatter + content
  }

  writeFileSync(filePath, newContent, 'utf-8')
  return { updated: true, title, description }
}

/**
 * Recursively find all MDX/MD files in a directory
 */
function findMarkdownFiles(dir: string, baseDir: string = dir): string[] {
  const files: string[] = []
  
  if (!existsSync(dir)) {
    return files
  }
  
  const entries = readdirSync(dir, { withFileTypes: true })
  
  for (const entry of entries) {
    const fullPath = join(dir, entry.name)
    
    if (entry.isDirectory()) {
      files.push(...findMarkdownFiles(fullPath, baseDir))
    } else if (entry.isFile() && /\.(mdx|md)$/.test(entry.name)) {
      files.push(relative(baseDir, fullPath))
    }
  }
  
  return files
}

/**
 * Main function to process all MDX files
 */
export function generateSEOMetadata(dryRun: boolean = false) {
  const pagesDir = join(process.cwd(), 'pages')

  if (!existsSync(pagesDir)) {
    console.error(`Pages directory not found: ${pagesDir}`)
    return
  }

  const files = findMarkdownFiles(pagesDir, pagesDir)

  console.log(`Found ${files.length} markdown files`)

  const results: Array<{ file: string; updated: boolean; title: string; description: string }> = []

  for (const file of files) {
    const filePath = join(pagesDir, file)
    const result = processFile(filePath, dryRun)
    results.push({ file, ...result })

    if (result.updated) {
      console.log(`✓ ${file}`)
      console.log(`  Title: ${result.title}`)
      console.log(`  Description: ${result.description.substring(0, 80)}...`)
    }
  }

  const updatedCount = results.filter((r) => r.updated).length
  console.log(`\n${updatedCount} files ${dryRun ? 'would be' : 'were'} updated`)

  return results
}

// Run if called directly
import { fileURLToPath } from 'node:url'
const __filename = fileURLToPath(import.meta.url)
if (process.argv[1] === __filename || process.argv[1]?.includes('generate-seo-metadata.ts')) {
  const dryRun = process.argv.includes('--dry-run')
  generateSEOMetadata(dryRun)
}

