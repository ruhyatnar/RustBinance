import { describe, expect, it } from 'vitest';
import {
	decimalDirectionIcon,
	decimalSign,
	formatDecimalString,
	formatDuration,
	formatUtcTime
} from './decimal';

describe('formatDecimalString (pure string, no parseFloat)', () => {
	it('preserves high-precision values that IEEE-754 float would corrupt', () => {
		expect(formatDecimalString('123456789012345.12345678')).toBe('123,456,789,012,345.12345678');
		expect(formatDecimalString('0.00000125')).toBe('0.00000125');
	});

	it('normalizes negative zero to positive zero', () => {
		expect(formatDecimalString('-0.00')).toBe('0.00');
		expect(formatDecimalString('-0')).toBe('0');
		expect(decimalSign('-0.00')).toBe(0);
	});

	it('supports signed prefix, fixed decimals truncation/padding, and suffix', () => {
		expect(formatDecimalString('0.82', { signed: true, suffix: '%' })).toBe('+0.82%');
		expect(formatDecimalString('-1.1', { decimals: 2, signed: true, suffix: '%' })).toBe('-1.10%');
		expect(formatDecimalString('68120.5', { decimals: 2 })).toBe('68,120.50');
		expect(formatDecimalString('3120.4099', { decimals: 2 })).toBe('3,120.40');
	});

	it('returns direction icons for color-blind accessibility', () => {
		expect(decimalDirectionIcon('1.9')).toBe('▲');
		expect(decimalDirectionIcon('-0.3')).toBe('▼');
		expect(decimalDirectionIcon('0.00')).toBe('•');
	});

	it('formats UTC timestamps and durations cleanly', () => {
		expect(formatUtcTime(1790000000000)).toMatch(/^\d{2}:\d{2}:\d{2}$/);
		expect(formatDuration(15420)).toBe('4h 17m');
		expect(formatDuration(45)).toBe('45s');
	});
});
