<script lang="ts">
	import { onMount } from 'svelte';
	import { botStream } from '$lib/api/stream.svelte';
	import type { TradeRecord } from '$lib/api/types';
	import KpiCard from '$lib/components/ui/KpiCard.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import Panel from '$lib/components/ui/Panel.svelte';
	import {
		decimalDirectionIcon,
		decimalSign,
		formatDecimalString,
		formatUtcTime
	} from '$lib/format/decimal';

	const snap = $derived(botStream.snapshot);
	let trades = $state<TradeRecord[]>([]);

	const daySign = $derived(snap ? decimalSign(snap.risk.day_pnl_pct) : 0);
	const weekSign = $derived(snap ? decimalSign(snap.risk.week_pnl_pct) : 0);

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

<div class="space-y-5">
	<PageHeader title="Performance" subtitle="Account equity metrics and closed trade history" />

	{#if snap}
		<div class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
			<KpiCard
				label="Equity"
				value={formatDecimalString(snap.risk.equity, { decimals: 2 })}
				sublabel="Account Aggregate (USDT)"
			/>
			<KpiCard
				label="Day PnL"
				value={formatDecimalString(snap.risk.day_pnl_pct, { signed: true, suffix: '%' })}
				icon={decimalDirectionIcon(snap.risk.day_pnl_pct)}
				tone={daySign > 0 ? 'positive' : daySign < 0 ? 'danger' : 'neutral'}
			/>
			<KpiCard
				label="Week PnL"
				value={formatDecimalString(snap.risk.week_pnl_pct, { signed: true, suffix: '%' })}
				icon={decimalDirectionIcon(snap.risk.week_pnl_pct)}
				tone={weekSign > 0 ? 'positive' : weekSign < 0 ? 'danger' : 'neutral'}
			/>
			<KpiCard
				label="Drawdown"
				value={formatDecimalString(snap.risk.drawdown_pct, { suffix: '%' })}
				tone="warning"
			/>
		</div>
	{/if}

	<Panel title={`Recent Trades (${trades.length})`} padding="none">
		{#if trades.length === 0}
			<p class="py-8 text-center font-mono text-xs text-ink-dim">No closed trades recorded yet.</p>
		{:else}
			<div class="overflow-x-auto">
				<table class="w-full text-left font-mono text-xs tabular-nums">
					<thead class="border-b border-hairline-subtle bg-surface-inset/70 text-[10px] tracking-[0.08em] text-ink-muted uppercase">
						<tr>
							<th class="px-4 py-2.5">Time (UTC)</th>
							<th class="px-3 py-2.5">Venue</th>
							<th class="px-3 py-2.5">Symbol</th>
							<th class="px-3 py-2.5">Price</th>
							<th class="px-3 py-2.5">Qty</th>
							<th class="px-4 py-2.5 text-right">Realized PnL</th>
						</tr>
					</thead>
					<tbody class="divide-y divide-hairline-subtle">
						{#each trades as t, i (`${t.symbol}-${t.ts_ms}-${i}`)}
							{@const pnlSign = t.realized_pnl ? decimalSign(t.realized_pnl) : 0}
							<tr class="transition hover:bg-surface-elevated/60">
								<td class="px-4 py-2.5 text-ink-dim">{formatUtcTime(t.ts_ms)}</td>
								<td class="px-3 py-2.5 text-ink-muted uppercase">{t.venue ?? 'spot'}</td>
								<td class="px-3 py-2.5 font-semibold text-ink-strong">{t.symbol}</td>
								<td class="px-3 py-2.5 text-ink">{formatDecimalString(t.price)}</td>
								<td class="px-3 py-2.5 text-ink">{formatDecimalString(t.qty)}</td>
								<td
									class={`px-4 py-2.5 text-right font-semibold ${
										pnlSign > 0
											? 'text-positive'
											: pnlSign < 0
												? 'text-danger'
												: 'text-ink-muted'
									}`}
								>
									{#if t.realized_pnl}
										{decimalDirectionIcon(t.realized_pnl)}
										{formatDecimalString(t.realized_pnl, { signed: true })}
									{:else}
										—
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</Panel>
</div>
