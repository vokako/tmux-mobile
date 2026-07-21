const MIN_TEXT_CONTRAST = 4.5;
const BG_CLASH_RATIO_DARK = 4.5;
const BG_CLASH_RATIO_LIGHT = 1.8;
const BG_BLEND_RATIO_LIGHT = 1.15;
const HSL_L_BG_DARK = 0.30;
const HSL_L_BG_LIGHT = 0.75;

type Rgb = [number, number, number];
type Hsl = [number, number, number];
// The xterm theme subset we read: fg/bg plus the 16 ANSI palette entries.
export type AnsiTheme = { foreground: string; background: string } & Record<string, string>;

const ANSI_KEYS = [
  'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
  'brightBlack', 'brightRed', 'brightGreen', 'brightYellow',
  'brightBlue', 'brightMagenta', 'brightCyan', 'brightWhite',
];

function hexToRgb(hex: string): Rgb {
  const value = hex.replace('#', '');
  return [0, 2, 4].map(offset => Number.parseInt(value.slice(offset, offset + 2), 16)) as Rgb;
}

function toLinearChannel(value: number): number {
  const channel = value / 255;
  return channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
}

export function relativeLuminance([red, green, blue]: Rgb): number {
  return 0.2126 * toLinearChannel(red) + 0.7152 * toLinearChannel(green) + 0.0722 * toLinearChannel(blue);
}

export function contrastRatio(first: Rgb, second: Rgb): number {
  const high = Math.max(relativeLuminance(first), relativeLuminance(second));
  const low = Math.min(relativeLuminance(first), relativeLuminance(second));
  return (high + 0.05) / (low + 0.05);
}

function rgbToHsl([red, green, blue]: Rgb): Hsl {
  const r = red / 255;
  const g = green / 255;
  const b = blue / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const lightness = (max + min) / 2;
  if (max === min) return [0, 0, lightness];

  const delta = max - min;
  const saturation = lightness > 0.5 ? delta / (2 - max - min) : delta / (max + min);
  let hue: number;
  if (max === r) hue = ((g - b) / delta + (g < b ? 6 : 0)) / 6;
  else if (max === g) hue = ((b - r) / delta + 2) / 6;
  else hue = ((r - g) / delta + 4) / 6;
  return [hue, saturation, lightness];
}

function hslToRgb([hue, saturation, lightness]: Hsl): Rgb {
  if (saturation === 0) {
    const value = Math.round(lightness * 255);
    return [value, value, value];
  }
  const hueToChannel = (low: number, high: number, position: number): number => {
    let p = position;
    if (p < 0) p += 1;
    if (p > 1) p -= 1;
    if (p < 1 / 6) return low + (high - low) * 6 * p;
    if (p < 1 / 2) return high;
    if (p < 2 / 3) return low + (high - low) * (2 / 3 - p) * 6;
    return low;
  };
  const high = lightness < 0.5
    ? lightness * (1 + saturation)
    : lightness + saturation - lightness * saturation;
  const low = 2 * lightness - high;
  return [hue + 1 / 3, hue, hue - 1 / 3]
    .map(position => Math.round(hueToChannel(low, high, position) * 255)) as Rgb;
}

function indexedColor(index: number, palette: Rgb[]): Rgb {
  if (index < 16) return palette[index]!; // index clamped to 0..255 by caller
  if (index >= 232) {
    const value = (index - 232) * 10 + 8;
    return [value, value, value];
  }
  const offset = index - 16;
  return [
    Math.floor(offset / 36) * 51,
    Math.floor((offset % 36) / 6) * 51,
    (offset % 6) * 51,
  ];
}

function adaptBackground(rgb: Rgb, terminalBackground: Rgb, dark: boolean): Rgb {
  const backgroundLuminance = relativeLuminance(rgb);
  const terminalLuminance = relativeLuminance(terminalBackground);
  const [hue, saturation] = rgbToHsl(rgb);
  if (dark) {
    const ratio = (backgroundLuminance + 0.05) / (terminalLuminance + 0.05);
    return ratio > BG_CLASH_RATIO_DARK
      ? hslToRgb([hue, saturation, HSL_L_BG_DARK])
      : rgb;
  }
  const ratio = (terminalLuminance + 0.05) / (backgroundLuminance + 0.05);
  return ratio > BG_CLASH_RATIO_LIGHT || ratio < BG_BLEND_RATIO_LIGHT
    ? hslToRgb([hue, saturation, HSL_L_BG_LIGHT])
    : rgb;
}

function ensureTextContrast(foreground: Rgb, background: Rgb): Rgb {
  if (contrastRatio(foreground, background) >= MIN_TEXT_CONTRAST) return foreground;

  const [hue, saturation, startLightness] = rgbToHsl(foreground);
  const findPassingLightness = (endpoint: number): number | null => {
    const endpointColor = hslToRgb([hue, saturation, endpoint]);
    if (contrastRatio(endpointColor, background) < MIN_TEXT_CONTRAST) return null;
    let passing = endpoint;
    let failing = startLightness;
    for (let iteration = 0; iteration < 14; iteration++) {
      const candidateLightness = (passing + failing) / 2;
      const candidate = hslToRgb([hue, saturation, candidateLightness]);
      if (contrastRatio(candidate, background) >= MIN_TEXT_CONTRAST) passing = candidateLightness;
      else failing = candidateLightness;
    }
    return passing;
  };
  const candidates = [findPassingLightness(0), findPassingLightness(1)]
    .filter((lightness): lightness is number => lightness != null);
  const targetLightness = candidates.reduce((best, lightness) => (
    Math.abs(lightness - startLightness) < Math.abs(best - startLightness) ? lightness : best
  ));
  return hslToRgb([hue, saturation, targetLightness]);
}

function trueColorSequence(role: number, rgb: Rgb): string {
  return `\x1b[${role};2;${rgb[0]};${rgb[1]};${rgb[2]}m`;
}

function readExtendedColor(params: number[], offset: number, palette: Rgb[]): { rgb: Rgb; consumed: number } | null {
  const mode = params[offset + 1];
  if (mode === 2 && params.length >= offset + 5) {
    return { rgb: params.slice(offset + 2, offset + 5).map(value => Math.max(0, Math.min(255, value))) as Rgb, consumed: 5 };
  }
  if (mode === 5 && params.length >= offset + 3) {
    return { rgb: indexedColor(Math.max(0, Math.min(255, params[offset + 2]!)), palette), consumed: 3 };
  }
  return null;
}

export function adaptAnsiColors(text: string, theme: AnsiTheme): string {
  const terminalForeground = hexToRgb(theme.foreground);
  const terminalBackground = hexToRgb(theme.background);
  const palette = ANSI_KEYS.map(key => hexToRgb(theme[key]!));
  const dark = relativeLuminance(terminalBackground) < 0.5;
  const state: { foreground: Rgb | null; background: Rgb | null; reverse: boolean } = { foreground: null, background: null, reverse: false };

  return text.replace(/\x1b\[([0-9;]*)m/g, (sequence, body: string) => {
    const params = body === '' ? [0] : body.split(';').map((value: string) => Number.parseInt(value || '0', 10));
    let colorsChanged = false;
    let fullReset = false;
    for (let index = 0; index < params.length; index++) {
      const param = params[index]!;
      if (param === 0) {
        state.foreground = null;
        state.background = null;
        state.reverse = false;
        colorsChanged = true;
        fullReset = true;
      } else if (param === 7) {
        state.reverse = true;
        colorsChanged = true;
      } else if (param === 27) {
        state.reverse = false;
        colorsChanged = true;
      } else if (param === 39) {
        state.foreground = null;
        colorsChanged = true;
      } else if (param === 49) {
        state.background = null;
        colorsChanged = true;
      } else if (param >= 30 && param <= 37) {
        state.foreground = palette[param - 30]!;
        colorsChanged = true;
      } else if (param >= 90 && param <= 97) {
        state.foreground = palette[param - 90 + 8]!;
        colorsChanged = true;
      } else if (param >= 40 && param <= 47) {
        state.background = palette[param - 40]!;
        colorsChanged = true;
      } else if (param >= 100 && param <= 107) {
        state.background = palette[param - 100 + 8]!;
        colorsChanged = true;
      } else if (param === 38 || param === 48) {
        const color = readExtendedColor(params, index, palette);
        if (color) {
          if (param === 38) state.foreground = color.rgb;
          else state.background = color.rgb;
          index += color.consumed - 1;
          colorsChanged = true;
        }
      }
    }
    if (!colorsChanged) return sequence;
    if (!state.foreground && !state.background && !state.reverse) {
      return fullReset ? sequence : sequence + '\x1b[39;49m';
    }

    const rawNormalForeground = state.foreground || terminalForeground;
    const rawNormalBackground = state.background || terminalBackground;
    const rawDisplayForeground = state.reverse ? rawNormalBackground : rawNormalForeground;
    const rawDisplayBackground = state.reverse ? rawNormalForeground : rawNormalBackground;
    const displayBackgroundIsExplicit = state.reverse || state.background != null;
    const displayBackground = displayBackgroundIsExplicit
      ? adaptBackground(rawDisplayBackground, terminalBackground, dark)
      : terminalBackground;
    const displayForeground = ensureTextContrast(rawDisplayForeground, displayBackground);
    const normalForeground = state.reverse ? displayBackground : displayForeground;
    const normalBackground = state.reverse ? displayForeground : displayBackground;
    return sequence + trueColorSequence(38, normalForeground) + trueColorSequence(48, normalBackground);
  });
}
