<script lang="ts">
	import type { MarketTickSummary } from '$lib/api/types';
	import { formatDecimalString, formatUtcTime } from '$lib/format/decimal';

	interface Props {
		ticks: MarketTickSummary[];
	}

	let { ticks }: Props = $props();
</script>

<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
	<div class="mb-3 flex items-center justify-between">
		<h2 class="text-xs font-bold tracking-wider text-slate-400 uppercase">
			Watchlist & Top-of-Book
		</h2>
		<span class="text-xs text-slate-500">{ticks.length} symbols</span>
	</div>

	{#if ticks.length === 0}
		<p class="py-4 text-center text-xs text-slate-500">No conflated market ticks yet.</p>
	{:else}
		<div class="overflow-x-auto">
			<table class="w-full text-left font-mono text-xs">
				<thead class="border-b border-slate-800 text-slate-400">
					<tr>
						<th class="py-1.5 pr-3">Symbol</th>
						<th class="py-1.5 pr-3">Venue</th>
						<th class="py-1.5 pr-3">Bid</th>
						<th class="py-1.5 pr-3">Ask</th>
						<th class="py-1.5 pr-3">Last</th>
						<th class="py-1.5 pr-3">Spread (bps)</th>
						<th class="py-1.5 text-right">Updated</th>
					</tr>
				</thead>
				<tbody class="divide-y divide-slate-800/80">
					{#each ticks as tick (`${tick.venue}:${tick.symbol}`)}
						<tr>
							<td class="py-1.5 pr-3 font-semibold text-white">{tick.symbol}</td>
							<td class="py-1.5 pr-3 uppercase text-slate-400">{tick.venue}</td>
							<td class="py-1.5 pr-3 text-emerald-300">{formatDecimalString(tick.bid)}</td>
							<td class="py-1.5 pr-3 text-red-300">{formatDecimalString(tick.ask)}</td>
							<td class="py-1.5 pr-3 text-slate-100">{formatDecimalString(tick.last)}</td>
							<td class="py-1.5 pr-3 text-amber-300">{formatDecimalString(tick.spread_bps)}</td>
							<td class="py-1.5 text-right text-slate-500">{formatUtcTime(tick.ts_ms)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</section>
