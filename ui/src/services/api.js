export async function getRoster(token) {
  const headers = new Headers({ Authorization: `Bearer ${btoa(token)}` });
  const response = await window.fetch('/api/roster', { headers });
  return await response.json();
}
