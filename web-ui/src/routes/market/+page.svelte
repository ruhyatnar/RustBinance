<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import Candles from '$lib/charts/Candles.svelte';
	import DepthLadder from '$lib/components/DepthLadder.svelte';

	let symbol = $state('BTCUSDT');
	let tf = $state('15m');
</script>

<div class="space-y-4">
	<div class="flex flex-wrap items-center gap-2 text-xs">
		<input
			bind:value={symbol}
			class="rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5 font-mono uppercase text-white"
			placeholder="BTCUSDT"
		/>
		<select bind:value={tf} class="rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-white">
			<option value="5m">5m</option>
			<option value="15m">15m</option>
			<option value="1h">1h</option>
			<option value="4h">4h</option>
		</select>
	</div>

	<Candles {symbol} timeframe={tf} bars={[]} />
	<DepthLadder ticks={botStream.marketTicks} />
</div>
