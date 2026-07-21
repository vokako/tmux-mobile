export function normalizeFontFamily(name: string | null | undefined): string {
  return (name || '').trim().replace(/['"]/g, '');
}

export function localFontSource(name: string | null | undefined): string {
  const family = normalizeFontFamily(name).replace(/\\/g, '\\\\');
  return `local("${family}")`;
}
