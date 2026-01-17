/**  
 * Vercel Edge Middleware for tracking AI crawlers.  
 */

const AI_CRAWLERS = [  
  'GPTBot',  
  'OAI-SearchBot',  
  'ChatGPT-User',  
  'anthropic-ai',  
  'ClaudeBot',  
  'claude-web',  
  'PerplexityBot',  
  'Perplexity-User',  
  'Google-Extended',  
  'Googlebot',  
  'Bingbot',  
  'Amazonbot',  
  'Applebot',  
  'Applebot-Extended',  
  'FacebookBot',  
  'meta-externalagent',  
  'LinkedInBot',  
  'Bytespider',  
  'DuckAssistBot',  
  'cohere-ai',  
  'AI2Bot',  
  'CCBot',  
  'Diffbot',  
  'omgili',  
  'Timpibot',  
  'YouBot',  
  'MistralAI-User',  
  'GoogleAgent-Mariner',  
]

export const config = {  
  matcher: [  
    '/((?!_next/static|_next/image|favicon.ico|.*\\.(?:png|jpg|jpeg|gif|svg|ico|webp)$).*)',  
  ],  
}

export default async function middleware(request: Request) {  
  const ua = request.headers.get('user-agent') || ''  
  const matchedCrawler = AI_CRAWLERS.find((crawler) => ua.includes(crawler))

  if (matchedCrawler) {  
    const url = new URL(request.url)  
      
    // For Vercel Edge Middleware, env vars need to be exposed  
    const posthogKey = 'phc_aNlTw2xAUQKd9zTovXeYheEUpQpEhplehCK5r1e31HR'
    const posthogHost = 'https://us.i.posthog.com'

    if (posthogKey) {  
      const ip = request.headers.get('x-forwarded-for')?.split(',')[0]?.trim()

      const event = {  
        api_key: posthogKey,  
        event: 'crawler_pageview',  
        distinct_id: `crawler_${matchedCrawler}`,  
        properties: {  
          crawler_name: matchedCrawler,  
          user_agent: ua,  
          path: url.pathname,  
          $current_url: request.url,  
          $ip: ip,  
        },  
        timestamp: new Date().toISOString(),  
      }

      // IMPORTANT: await the fetch so it completes before middleware exits  
      try {  
        const response = await fetch(`${posthogHost}/capture/`, {  
          method: 'POST',  
          headers: { 'Content-Type': 'application/json' },  
          body: JSON.stringify(event),  
        })  
          
        // Log response in non-production  
        if (!response.ok) {  
          console.error('PostHog capture failed:', response.status, await response.text())  
        }  
      } catch (error) {  
        console.error('PostHog capture error:', error)  
      }  
    } else {  
      console.warn('something is off')  
    }  
  }

  // Continue to the page  
  return undefined  
}