<script lang="ts">
	import { onMount } from 'svelte';
	import { CandlestickSeries, createChart, type UTCTimestamp } from 'lightweight-charts';
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

		const mapped = bars.map((b) => ({
			time: Math.floor(b.time / 1000) as UTCTimestamp,
			open: Number(b.open),
			high: Number(b.high),
			low: Number(b.low),
			close: Number(b.close)
		}));

		if (mapped.length > 0) {
			series.setData(mapped);
			chart.timeScale().fitContent();
		}

		if (entryPrice) {
			series.createPriceLine({
				price: Number(entryPrice),
				color: '#50d8e9',
				lineWidth: 1,
				title: 'Entry'
			});
		}
		if (stopPrice) {
			series.createPriceLine({
				price: Number(stopPrice),
				color: '#ffb4ab',
				lineWidth: 1,
				title: 'SL'
			});
		}
		if (tp1Price) {
			series.createPriceLine({
				price: Number(tp1Price),
				color: '#34d399',
				lineWidth: 1,
				title: 'TP1'
			});
		}

		const ro = new ResizeObserver(() => {
			if (container) {
				chart.applyOptions({ width: container.clientWidth });
			}
		});
		ro.observe(container);

		return () => {
			ro.disconnect();
			chart.remove();
		};
	});
</script>

<Panel title={`Chart · ${symbol}`} subtitle={`${timeframe} · ${bars.length} bars`} dotGrid={true}>
	<div
		bind:this={container}
		style={`height: ${height}px`}
		class="w-full overflow-hidden rounded border border-hairline-subtle bg-surface"
	></div>
</Panel>
