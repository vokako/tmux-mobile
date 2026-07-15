export function normalizeFontFamily(name) {
  return (name || '').trim().replace(/['"]/g, '');
}

export function localFontSource(name) {
  const family = normalizeFontFamily(name).replace(/\\/g, '\\\\');
  return `local("${family}")`;
}
