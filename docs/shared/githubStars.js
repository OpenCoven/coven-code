export const GITHUB_STARS_API = 'https://api.github.com/repos/OpenCoven/coven-code';
export const GITHUB_STARS_ACCEPT = 'application/vnd.github.v3+json';
export const STARS_CACHE_TTL_SECONDS = 60;
export const STARS_STALE_REVALIDATE_SECONDS = STARS_CACHE_TTL_SECONDS * 5;

export function parseGithubStarCount(data) {
  const count = data?.stargazers_count;
  return typeof count === 'number' ? count : null;
}

export async function fetchGithubStarCount(fetchImpl = fetch, userAgent = 'coven-code-docs') {
  const headers = { Accept: GITHUB_STARS_ACCEPT };
  if (userAgent) headers['User-Agent'] = userAgent;

  const response = await fetchImpl(GITHUB_STARS_API, { headers });
  if (!response.ok) return null;

  return parseGithubStarCount(await response.json());
}
