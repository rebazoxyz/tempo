/**
 * Quick script to test OG image URLs for pages
 * Run with: tsx scripts/test-og-images.ts
 */

import { readFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { extractTitle, extractDescription } from './generate-seo-metadata'

const baseUrl = 'http://localhost:5173'
const logoUrl = `${baseUrl}/lockup-light.svg`

function getPageMetadata(filePath: string) {
  const content = readFileSync(filePath, 'utf-8')
  
  // Extract frontmatter
  const frontmatterMatch = content.match(/^---\n([\s\S]*?)\n---\n/)
  let title = ''
  let description = ''
  
  const frontmatter = frontmatterMatch?.[1]
  if (frontmatter) {
    const titleMatch = frontmatter.match(/title:\s*(.+)/)
    const descMatch = frontmatter.match(/description:\s*(.+)/)
    
    if (titleMatch?.[1]) {
      title = titleMatch[1].trim().replace(/^["']|["']$/g, '')
    }
    if (descMatch?.[1]) {
      description = descMatch[1].trim().replace(/^["']|["']$/g, '')
    }
  }
  
  // Fallback to extraction if not in frontmatter
  if (!title) {
    title = extractTitle(content, filePath)
  }
  if (!description) {
    description = extractDescription(content)
  }
  
  return { title, description }
}

function generateOGImageUrl(title: string, description: string) {
  const params = new URLSearchParams({
    logo: logoUrl,
    title: title,
    description: description,
  })
  return `https://vocs.dev/api/og?${params.toString()}`
}

// Test a specific page
const testPage = process.argv[2] || 'pages/learn/use-cases/tokenized-deposits.mdx'
const filePath = join(process.cwd(), testPage)

if (!existsSync(filePath)) {
  console.error(`File not found: ${filePath}`)
  process.exit(1)
}

const { title, description } = getPageMetadata(filePath)
const ogImageUrl = generateOGImageUrl(title, description)

console.log('\n📄 Page:', testPage)
console.log('📝 Title:', title)
console.log('📄 Description:', description.substring(0, 80) + '...')
console.log('\n🖼️  OG Image URL:')
console.log(ogImageUrl)
console.log('\n💡 Copy the URL above and paste it in your browser to see the generated image!')
console.log('   Note: The Vocs API may not work with localhost URLs. For production, use your deployed URL.\n')

