export async function peers(token) {
  const headers = new Headers({ Authorization: `Bearer ${btoa(token)}` });
  const response = await window.fetch('/api/peers', { headers });
  return await response.json();
}
