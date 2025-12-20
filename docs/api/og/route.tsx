import { ImageResponse } from '@vercel/og'

export const runtime = 'edge'

export async function GET(request: Request) {
  try {
    const { searchParams } = new URL(request.url)

    // Get query parameters with defaults
    const title = searchParams.get('title') || 'Documentation • Tempo'
    const description =
      searchParams.get('description') ||
      'Documentation for Tempo testnet and protocol specifications'
    const logoUrl = searchParams.get('logo') || ''

    // Truncate text to fit within image bounds
    const truncatedTitle =
      title.length > 60 ? `${title.slice(0, 57)}...` : title
    const truncatedDescription =
      description.length > 120 ? `${description.slice(0, 117)}...` : description

    return new ImageResponse(
      (
        <div
          style={{
            height: '100%',
            width: '100%',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: '#ffffff',
            backgroundImage:
              'linear-gradient(to bottom right, #ffffff 0%, #f8f9fa 100%)',
            fontFamily: 'system-ui, -apple-system, sans-serif',
            padding: '80px 120px',
          }}
        >
          {/* Logo */}
          {logoUrl && (
            <div
              style={{
                display: 'flex',
                marginBottom: '40px',
              }}
            >
              <img
                src={logoUrl}
                alt="Tempo"
                width="148"
                height="35"
                style={{
                  objectFit: 'contain',
                }}
              />
            </div>
          )}

          {/* Title */}
          <div
            style={{
              display: 'flex',
              fontSize: '64px',
              fontWeight: '700',
              lineHeight: '1.2',
              color: '#000000',
              textAlign: 'center',
              marginBottom: '24px',
              maxWidth: '960px',
            }}
          >
            {truncatedTitle}
          </div>

          {/* Description */}
          <div
            style={{
              display: 'flex',
              fontSize: '32px',
              fontWeight: '400',
              lineHeight: '1.4',
              color: '#666666',
              textAlign: 'center',
              maxWidth: '960px',
            }}
          >
            {truncatedDescription}
          </div>
        </div>
      ),
      {
        width: 1200,
        height: 630,
      },
    )
  } catch (error) {
    console.error('OG image generation error:', error)
    // Return a simple error image
    return new ImageResponse(
      (
        <div
          style={{
            height: '100%',
            width: '100%',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: '#ffffff',
            fontSize: '32px',
            color: '#666666',
          }}
        >
          Documentation • Tempo
        </div>
      ),
      {
        width: 1200,
        height: 630,
      },
    )
  }
}
