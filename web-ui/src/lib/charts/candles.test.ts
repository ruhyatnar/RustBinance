import { render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const setDataMock = vi.fn();
const createPriceLineMock = vi.fn((opts: unknown) => ({ opts }));
const removePriceLineMock = vi.fn();
const fitContentMock = vi.fn();
const applyOptionsMock = vi.fn();
const removeChartMock = vi.fn();

vi.mock('lightweight-charts', () => ({
	CandlestickSeries: 'CandlestickSeries',
	createChart: vi.fn(() => ({
		addSeries: vi.fn(() => ({
			setData: setDataMock,
			createPriceLine: createPriceLineMock,
			removePriceLine: removePriceLineMock
		})),
		timeScale: vi.fn(() => ({ fitContent: fitContentMock })),
		applyOptions: applyOptionsMock,
		remove: removeChartMock
	}))
}));

import Candles from './Candles.svelte';

describe('Candles reactive chart updates', () => {
	const originalRO = globalThis.ResizeObserver;

	beforeEach(() => {
		vi.clearAllMocks();
		globalThis.ResizeObserver = class {
			observe() {}
			disconnect() {}
			unobserve() {}
		} as unknown as typeof ResizeObserver;
	});

	afterEach(() => {
		globalThis.ResizeObserver = originalRO;
	});

	it('updates price lines and bar data reactively when props change', async () => {
		const { rerender } = render(Candles, {
			props: {
				symbol: 'ETHUSDT',
				timeframe: '1h',
				bars: [],
				entryPrice: '3120.40',
				stopPrice: '3098.00',
				tp1Price: '3200.00'
			}
		});

		expect(createPriceLineMock).toHaveBeenCalledTimes(3);

		await rerender({
			symbol: 'BTCUSDT',
			timeframe: '1h',
			bars: [
				{ time: 1_700_000_000_000, open: '60000', high: '61000', low: '59500', close: '60500' }
			],
			entryPrice: null,
			stopPrice: null,
			tp1Price: null
		});

		expect(removePriceLineMock).toHaveBeenCalledTimes(3);
		expect(setDataMock).toHaveBeenCalledWith([
			{ time: 1_700_000_000, open: 60000, high: 61000, low: 59500, close: 60500 }
		]);
	});
});
