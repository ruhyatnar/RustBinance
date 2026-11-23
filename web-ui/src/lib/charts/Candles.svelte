<script lang="ts">
	import { onMount } from 'svelte';
	import { CandlestickSeries, createChart, type UTCTimestamp } from 'lightweight-charts';
	import type { KlineBar } from '$lib/api/types';

	interface Props {
		symbol: string;
		timeframe?: string;
		bars: KlineBar[];
		entryPrice?: string | null;
		stopPrice?: string | null;
		tp1Price?: string | null;
	}

	let {
		symbol,
		timeframe = '1h',
		bars,
		entryPrice = null,
		stopPrice = null,
		tp1Price = null
	}: Props = $props();

	let container: HTMLDivElement | undefined = $state();

	onMount(() => {
		if (!container) return;
		const chart = createChart(container, {
			layout: {
				background: { color: '#0f172a' },
				textColor: '#94a3b8'
			},
			grid: {
				vertLines: { color: '#1e293b' },
				horzLines: { color: '#1e293b' }
			},
			width: container.clientWidth || 600,
			height: 280
		});

		const series = chart.addSeries(CandlestickSeries, {
			upColor: '#10b981',
			downColor: '#ef4444',
			borderVisible: false,
			wickUpColor: '#10b981',
			wickDownColor: '#ef4444'
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
				color: '#38bdf8',
				lineWidth: 1,
				title: 'Entry'
			});
		}
		if (stopPrice) {
			series.createPriceLine({
				price: Number(stopPrice),
				color: '#ef4444',
				lineWidth: 1,
				title: 'SL'
			});
		}
		if (tp1Price) {
			series.createPriceLine({
				price: Number(tp1Price),
				color: '#10b981',
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

<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
	<div class="mb-2 flex items-center justify-between text-xs">
		<div class="font-bold tracking-wider text-slate-300 uppercase">
			Chart · {symbol} ({timeframe})
		</div>
		<span class="text-slate-500">{bars.length} bars</span>
	</div>
	<div bind:this={container} class="h-[280px] w-full overflow-hidden rounded-lg"></div>
</section>
