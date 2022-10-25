export function getCookieValue(lookupKey) {
  for (const cookie of document.cookie.split(";")) {
    const [key, value] = cookie.split("=");
    if (lookupKey === key) {
      return value;
    }
  }
  return undefined;
}

export function setCookieValue(key, value) {
  document.cookie = `${key}=${encodeURIComponent(value)};`;
}
