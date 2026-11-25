<script lang="ts">
	import type { MarketTickSummary } from '$lib/api/types';
	import Panel from '$lib/components/ui/Panel.svelte';
	import { formatDecimalString, formatUtcTime } from '$lib/format/decimal';

	interface Props {
		ticks: MarketTickSummary[];
	}

	let { ticks }: Props = $props();
</script>

<Panel title="Watchlist & Top-of-Book" subtitle={`${ticks.length} symbols`} padding="none">
	{#if ticks.length === 0}
		<p class="py-6 text-center font-mono text-xs text-ink-dim">No conflated market ticks yet.</p>
	{:else}
		<div class="overflow-x-auto">
			<table class="w-full text-left font-mono text-xs tabular-nums">
				<thead class="border-b border-hairline-subtle bg-surface-inset/70 text-[10px] tracking-[0.08em] text-ink-muted uppercase">
					<tr>
						<th class="px-4 py-2">Symbol</th>
						<th class="px-3 py-2">Venue</th>
						<th class="px-3 py-2">Bid</th>
						<th class="px-3 py-2">Ask</th>
						<th class="px-3 py-2">Last</th>
						<th class="px-3 py-2">Spread (bps)</th>
						<th class="px-4 py-2 text-right">Updated</th>
					</tr>
				</thead>
				<tbody class="divide-y divide-hairline-subtle">
					{#each ticks as tick (`${tick.venue}:${tick.symbol}`)}
						<tr class="transition hover:bg-surface-elevated/60">
							<td class="px-4 py-2 font-semibold text-ink-strong">{tick.symbol}</td>
							<td class="px-3 py-2 text-ink-muted uppercase">{tick.venue}</td>
							<td class="px-3 py-2 text-positive">{formatDecimalString(tick.bid)}</td>
							<td class="px-3 py-2 text-danger">{formatDecimalString(tick.ask)}</td>
							<td class="px-3 py-2 text-ink-strong">{formatDecimalString(tick.last)}</td>
							<td class="px-3 py-2 text-warning">{formatDecimalString(tick.spread_bps)}</td>
							<td class="px-4 py-2 text-right text-ink-dim">{formatUtcTime(tick.ts_ms)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</Panel>
