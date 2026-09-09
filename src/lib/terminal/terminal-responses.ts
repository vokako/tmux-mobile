// Bound malformed/incomplete input, not time: chunk boundaries may be arbitrarily
// far apart. Real xterm 6 DA/DSR replies are far shorter than this resource cap.
const MAX_CARRY = 512;
const ESC = '\x1b';

function isResponse(data: string): boolean {
  return /^\x1b\[[\?>=]?[\d;]*c$/.test(data)
    || /^\x1b\[\d+;\d+R$/.test(data)
    || /^\x1b\[\d+n$/.test(data);
}

/** One filter per xterm instance. Only an unfinished response candidate waits;
 * ordinary text and complete keyboard sequences pass through synchronously. */
export function createTerminalResponseFilter() {
  let pending = '';
  return {
    push(data: string, literal = false): string {
      // A paste is user data, even if it contains bytes shaped like a reply.
      if (literal || (!pending && !data.includes(ESC))) return data;
      let output = '';
      for (const char of data) {
        if (char === ESC) {
          output += pending;
          pending = ESC;
        } else if (!pending) {
          output += char;
        } else if (pending === ESC) {
          if (char === '[') pending += char;
          else { output += pending + char; pending = ''; }
        } else if ((char >= '0' && char <= '9') || char === ';'
          || (pending === ESC + '[' && (char === '?' || char === '>' || char === '='))) {
          pending += char;
          if (pending.length >= MAX_CARRY) { output += pending; pending = ''; }
        } else {
          const sequence = pending + char;
          if (!isResponse(sequence)) output += sequence;
          pending = '';
        }
      }
      return output;
    },
    reset(): void { pending = ''; },
  };
}
