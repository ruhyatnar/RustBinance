<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import Candles from '$lib/charts/Candles.svelte';
	import DepthLadder from '$lib/components/DepthLadder.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';

	let symbol = $state('BTCUSDT');
	let tf = $state('15m');
</script>

<div class="space-y-5">
	<PageHeader title="Market" subtitle="Multi-timeframe candlestick chart & conflated top-of-book">
		{#snippet actions()}
			<input
				bind:value={symbol}
				aria-label="Symbol"
				class="w-32 rounded border border-hairline bg-surface-inset px-2.5 py-1 font-mono text-xs text-ink-strong uppercase focus:border-accent focus:outline-none"
				placeholder="BTCUSDT"
			/>
			<select
				bind:value={tf}
				aria-label="Timeframe"
				class="rounded border border-hairline bg-surface-inset px-2.5 py-1 font-mono text-xs text-ink-strong focus:border-accent focus:outline-none"
			>
				<option value="5m">5m</option>
				<option value="15m">15m</option>
				<option value="1h">1h</option>
				<option value="4h">4h</option>
			</select>
		{/snippet}
	</PageHeader>

	<Candles {symbol} timeframe={tf} bars={[]} height={360} />
	<DepthLadder ticks={botStream.marketTicks} />
</div>
