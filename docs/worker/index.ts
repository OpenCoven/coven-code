import {
  fetchGithubStarCount,
  STARS_CACHE_TTL_SECONDS,
} from '../shared/githubStars.js';

export default {
  async fetch(request): Promise<Response> {
    const url = new URL(request.url);

    if (url.pathname === '/api/stars') {
      return handleStars(request);
    }

    return new Response(null, { status: 404 });
  },
} satisfies ExportedHandler;

async function handleStars(request: Request): Promise<Response> {
  const cache = caches.default;
  const cacheKey = new Request(new URL('/api/stars?v=3', request.url).toString());

  const cached = await cache.match(cacheKey);
  if (cached) return cached;

  try {
    const stars = await fetchGithubStarCount(fetch, 'coven-code-docs-worker');
    if (stars == null) {
      return Response.json(
        { stars: null },
        {
          status: 502,
          headers: {
            'Cache-Control': `public, max-age=${STARS_CACHE_TTL_SECONDS}`,
            'Access-Control-Allow-Origin': '*',
          },
        }
      );
    }

    const response = Response.json(
      { stars },
      {
        headers: {
          'Cache-Control': `public, max-age=${STARS_CACHE_TTL_SECONDS}`,
          'Access-Control-Allow-Origin': '*',
        },
      }
    );

    await cache.put(cacheKey, response.clone());
    return response;
  } catch {
    return Response.json(
      { stars: null },
      {
        status: 502,
        headers: {
          'Cache-Control': `public, max-age=${STARS_CACHE_TTL_SECONDS}`,
          'Access-Control-Allow-Origin': '*',
        },
      }
    );
  }
}
