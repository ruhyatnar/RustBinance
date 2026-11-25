<script lang="ts">
	import { onMount } from 'svelte';
	import {
		CandlestickSeries,
		createChart,
		type IChartApi,
		type IPriceLine,
		type ISeriesApi,
		type UTCTimestamp
	} from 'lightweight-charts';
	import type { KlineBar } from '$lib/api/types';
	import Panel from '$lib/components/ui/Panel.svelte';

	interface Props {
		symbol: string;
		timeframe?: string;
		bars: KlineBar[];
		entryPrice?: string | null;
		stopPrice?: string | null;
		tp1Price?: string | null;
		height?: number;
	}

	let {
		symbol,
		timeframe = '1h',
		bars,
		entryPrice = null,
		stopPrice = null,
		tp1Price = null,
		height = 280
	}: Props = $props();

	let container: HTMLDivElement | undefined = $state();
	let chartApi = $state<IChartApi | null>(null);
	let seriesApi = $state<ISeriesApi<'Candlestick'> | null>(null);
	let activePriceLines: IPriceLine[] = [];

	onMount(() => {
		if (!container || typeof ResizeObserver === 'undefined') return;
		const chart = createChart(container, {
			layout: {
				background: { color: '#101112' },
				textColor: '#9a9da3'
			},
			grid: {
				vertLines: { color: '#1b1c1e' },
				horzLines: { color: '#1b1c1e' }
			},
			width: container.clientWidth || 600,
			height
		});

		const series = chart.addSeries(CandlestickSeries, {
			upColor: '#34d399',
			downColor: '#f87171',
			borderVisible: false,
			wickUpColor: '#34d399',
			wickDownColor: '#f87171'
		});

		chartApi = chart;
		seriesApi = series;

		const ro = new ResizeObserver(() => {
			if (container && container.clientWidth > 0) {
				chart.applyOptions({ width: container.clientWidth });
			}
		});
		ro.observe(container);

		return () => {
			ro.disconnect();
			activePriceLines = [];
			seriesApi = null;
			chartApi = null;
			chart.remove();
		};
	});

	$effect(() => {
		const chart = chartApi;
		if (!chart) return;
		chart.applyOptions({ height });
	});

	$effect(() => {
		const chart = chartApi;
		const series = seriesApi;
		if (!chart || !series) return;

		const mapped = bars.map((b) => ({
			time: Math.floor(b.time / 1000) as UTCTimestamp,
			open: Number(b.open),
			high: Number(b.high),
			low: Number(b.low),
			close: Number(b.close)
		}));

		series.setData(mapped);
		if (mapped.length > 0) {
			chart.timeScale().fitContent();
		}
	});

	$effect(() => {
		const series = seriesApi;
		if (!series) return;

		for (const line of activePriceLines) {
			series.removePriceLine(line);
		}
		activePriceLines = [];

		const specs: Array<{ raw: string | null; color: string; title: string }> = [
			{ raw: entryPrice, color: '#50d8e9', title: 'Entry' },
			{ raw: stopPrice, color: '#ffb4ab', title: 'SL' },
			{ raw: tp1Price, color: '#34d399', title: 'TP1' }
		];

		for (const spec of specs) {
			if (!spec.raw) continue;
			const price = Number(spec.raw);
			if (!Number.isFinite(price)) continue;
			activePriceLines.push(
				series.createPriceLine({
					price,
					color: spec.color,
					lineWidth: 1,
					title: spec.title
				})
			);
		}
	});
</script>

<Panel title={`Chart · ${symbol}`} subtitle={`${timeframe} · ${bars.length} bars`} dotGrid={true}>
	<div
		bind:this={container}
		style={`height: ${height}px`}
		class="w-full overflow-hidden rounded border border-hairline-subtle bg-surface"
	></div>
</Panel>
