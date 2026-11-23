/**
 * Pure string-based decimal formatting utilities (§11.5).
 *
 * Monetary, quantity, PnL, and risk values arrive from `bot-ipc` as exact decimal
 * strings and MUST NOT be passed through `parseFloat` for precision display.
 */

const DECIMAL_RE = /^([+-])?(\d+)(?:\.(\d+))?$/;

export interface FormatDecimalOptions {
	/** Optional fixed number of fractional digits (truncates or zero-pads without float conversion). */
	decimals?: number;
	/** Prefix positive non-zero values with `+`. */
	signed?: boolean;
	/** Optional suffix (e.g. `%`, `R`, ` USDT`). */
	suffix?: string;
}

/**
 * Determine the exact sign of a decimal string (`-1`, `0`, or `1`) without `parseFloat`.
 */
export function decimalSign(raw: string): -1 | 0 | 1 {
	const trimmed = raw.trim();
	const m = DECIMAL_RE.exec(trimmed);
	if (!m) return 0;
	const signChar = m[1] ?? '+';
	const intPart = m[2];
	const fracPart = m[3] ?? '';
	const allZero = /^0+$/.test(intPart) && (fracPart === '' || /^0+$/.test(fracPart));
	if (allZero) return 0;
	return signChar === '-' ? -1 : 1;
}

/**
 * Color-blind accessible directional indicator (§11.1 #3).
 */
export function decimalDirectionIcon(raw: string): '▲' | '▼' | '•' {
	const sign = decimalSign(raw);
	if (sign > 0) return '▲';
	if (sign < 0) return '▼';
	return '•';
}

function addThousandsSeparators(digits: string): string {
	const normalized = digits.replace(/^0+(?=\d)/, '');
	let out = '';
	const len = normalized.length;
	for (let i = 0; i < len; i += 1) {
		if (i > 0 && (len - i) % 3 === 0) {
			out += ',';
		}
		out += normalized[i];
	}
	return out;
}

/**
 * Format an exact decimal string with thousands separators, optional sign, and optional
 * fixed fractional digits — purely via string manipulation (zero `parseFloat`).
 */
export function formatDecimalString(raw: string, opts: FormatDecimalOptions = {}): string {
	const trimmed = raw.trim();
	const m = DECIMAL_RE.exec(trimmed);
	if (!m) return raw;

	const sign = decimalSign(trimmed);
	const intFormatted = addThousandsSeparators(m[2]);
	let frac = m[3] ?? '';

	if (typeof opts.decimals === 'number' && opts.decimals >= 0) {
		if (opts.decimals === 0) {
			frac = '';
		} else if (frac.length > opts.decimals) {
			frac = frac.slice(0, opts.decimals);
		} else if (frac.length < opts.decimals) {
			frac = frac.padEnd(opts.decimals, '0');
		}
	}

	const prefix = sign < 0 ? '-' : sign > 0 && opts.signed ? '+' : '';
	const body = frac.length > 0 ? `${intFormatted}.${frac}` : intFormatted;
	return `${prefix}${body}${opts.suffix ?? ''}`;
}

/**
 * Format UTC epoch milliseconds as `HH:MM:SS` (UTC).
 */
export function formatUtcTime(tsMs: number): string {
	const d = new Date(tsMs);
	const hh = String(d.getUTCHours()).padStart(2, '0');
	const mm = String(d.getUTCMinutes()).padStart(2, '0');
	const ss = String(d.getUTCSeconds()).padStart(2, '0');
	return `${hh}:${mm}:${ss}`;
}

/**
 * Format duration in seconds into compact human-readable form (e.g. `4h 17m`, `45s`).
 */
export function formatDuration(seconds: number): string {
	const s = Math.max(0, Math.floor(seconds));
	if (s < 60) return `${s}s`;
	const mins = Math.floor(s / 60);
	if (mins < 60) return `${mins}m ${s % 60}s`;
	const hours = Math.floor(mins / 60);
	if (hours < 24) return `${hours}h ${mins % 60}m`;
	const days = Math.floor(hours / 24);
	return `${days}d ${hours % 24}h`;
}
