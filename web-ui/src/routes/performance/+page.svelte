<script lang="ts">
	import { onMount } from 'svelte';
	import { botStream } from '$lib/api/stream.svelte';
	import type { TradeRecord } from '$lib/api/types';
	import { decimalDirectionIcon, formatDecimalString, formatUtcTime } from '$lib/format/decimal';

	const snap = $derived(botStream.snapshot);
	let trades = $state<TradeRecord[]>([]);

	onMount(async () => {
		const client = botStream.getHttpClient();
		if (!client) return;
		try {
			const res = await client.trades();
			trades = res.trades;
		} catch {
			trades = [];
		}
	});
</script>

<div class="space-y-4">
	{#if snap}
		<div class="grid grid-cols-1 gap-3 font-mono text-xs sm:grid-cols-4">
			<div class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
				<div class="text-slate-400">Equity</div>
				<div class="mt-1 text-lg font-bold text-white">
					{formatDecimalString(snap.risk.equity, { decimals: 2 })}
				</div>
			</div>
			<div class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
				<div class="text-slate-400">Day PnL</div>
				<div class="mt-1 text-lg font-bold">
					{decimalDirectionIcon(snap.risk.day_pnl_pct)}
					{formatDecimalString(snap.risk.day_pnl_pct, { signed: true, suffix: '%' })}
				</div>
			</div>
			<div class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
				<div class="text-slate-400">Week PnL</div>
				<div class="mt-1 text-lg font-bold">
					{decimalDirectionIcon(snap.risk.week_pnl_pct)}
					{formatDecimalString(snap.risk.week_pnl_pct, { signed: true, suffix: '%' })}
				</div>
			</div>
			<div class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
				<div class="text-slate-400">Drawdown</div>
				<div class="mt-1 text-lg font-bold text-amber-300">
					{formatDecimalString(snap.risk.drawdown_pct, { suffix: '%' })}
				</div>
			</div>
		</div>
	{/if}

	<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
		<h2 class="mb-3 text-xs font-bold tracking-wider text-slate-400 uppercase">
			Recent Trades ({trades.length})
		</h2>
		{#if trades.length === 0}
			<p class="py-4 text-center text-xs text-slate-500">No closed trades recorded yet.</p>
		{:else}
			<div class="overflow-x-auto">
				<table class="w-full text-left font-mono text-xs">
					<thead class="border-b border-slate-800 text-slate-400">
						<tr>
							<th class="py-2 pr-3">Time (UTC)</th>
							<th class="py-2 pr-3">Venue</th>
							<th class="py-2 pr-3">Symbol</th>
							<th class="py-2 pr-3">Price</th>
							<th class="py-2 pr-3">Qty</th>
							<th class="py-2">Realized PnL</th>
						</tr>
					</thead>
					<tbody class="divide-y divide-slate-800/80">
						{#each trades as t, i (`${t.symbol}-${t.ts_ms}-${i}`)}
							<tr>
								<td class="py-2 pr-3 text-slate-400">{formatUtcTime(t.ts_ms)}</td>
								<td class="py-2 pr-3 uppercase">{t.venue ?? 'spot'}</td>
								<td class="py-2 pr-3 font-semibold text-white">{t.symbol}</td>
								<td class="py-2 pr-3">{formatDecimalString(t.price)}</td>
								<td class="py-2 pr-3">{formatDecimalString(t.qty)}</td>
								<td class="py-2">
									{t.realized_pnl ? formatDecimalString(t.realized_pnl, { signed: true }) : '—'}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</section>
</div>
